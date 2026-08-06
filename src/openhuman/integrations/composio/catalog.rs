//! Live Composio tool contracts: the catalog, the probe, and their caches.
//!
//! What a Composio action really accepts and really returns, sourced from
//! Composio itself rather than from a static curated list — the ground truth the
//! workflow builder authors against, and the enforcement gates validate against.
//!
//! Two sources, in priority order:
//!
//! 1. **The published schema.** [`fetch_live_toolkit_catalog`] reads Composio's
//!    own v3 `/tools` listing and derives a [`ToolContract`] per action.
//! 2. **A real response.** Many actions publish no `output_parameters` at all,
//!    so [`probe_tool_output_sample`] makes one bounded, READ-only, real call
//!    and derives the same hints from the actual value.
//!    [`apply_probe_override`] lets a probe win over a schema, because an
//!    observed response outranks a documented one.
//!
//! # Why this lives in `composio`, not in the workflow adapter seam
//!
//! It used to live in the seam, which put the dependency backwards: an
//! always-compiled domain reaching into a feature-gated adapter. That made the
//! adapter impossible to gate off, since `composio` would have followed it out
//! of the build. Everything here is Composio's own vocabulary — action slugs,
//! toolkits, the execute-response envelope, connection scope — so it belongs to
//! the domain that owns that vocabulary. The seam now imports from here.
//!
//! The vendor-neutral half of the work (walking a JSON Schema or a JSON value
//! for its primary array and its field names) is in
//! [`crate::openhuman::json_schema`], owned by neither side.

use serde_json::Value;

use super::client::{
    create_composio_client, direct_execute, direct_list_tools, ComposioClientKind,
};
use crate::openhuman::config::Config;
use crate::openhuman::json_schema::{
    compute_primary_array_path, compute_primary_array_path_from_value, response_fields_from_schema,
};

/// One Composio action's LIVE, ground-truth contract — the source of truth
/// [Part 1 of the systemic tool-contract fix] grounds the Workflow builder
/// against, replacing the old "guess a slug/arg/field/path and hope"
/// authoring flow.
///
/// Everything on this type comes straight from Composio's own v3 `/tools`
/// listing (`ComposioToolFunction` — `parameters`/`output_parameters`), never
/// from OpenHuman's static curated catalog: `required_args`/`input_schema`
/// are the action's real input contract, `output_fields`/`output_schema`/
/// `primary_array_path` are its real output contract. `is_curated` is the
/// ONE field that cross-references the static catalog — purely for ranking
/// (curated matches first in `search_tool_catalog`), never for filtering:
/// a real, uncurated action still produces a full `ToolContract`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolContract {
    /// The Composio action slug, e.g. `"GMAIL_SEND_EMAIL"`.
    pub slug: String,
    /// The lowercase toolkit slug this action belongs to, e.g. `"gmail"`.
    pub toolkit: String,
    /// Human-readable description shown to the model, when Composio
    /// publishes one for this action.
    pub description: Option<String>,
    /// Required top-level input argument names (`input_schema`'s
    /// `required` array). Empty when the action takes no required args —
    /// NOT the same as "schema unknown" (there is no such state here: an
    /// action always has SOME input schema, even if empty).
    pub required_args: Vec<String>,
    /// The action's full input JSON Schema, verbatim from Composio.
    pub input_schema: Option<Value>,
    /// Top-level output/response field names — empty when
    /// [`Self::output_schema`] is `None` (unknown) OR when it's `Some` but
    /// names no top-level properties; check `output_schema` to tell those
    /// two apart.
    ///
    /// **These name fields of the tool's PAYLOAD, not of the runtime
    /// envelope.** Composio's `output_parameters` (what [`Self::output_schema`]
    /// mirrors) describes the return value the provider hands back — the
    /// same value that ends up under `ComposioExecuteResponse.data` — NOT
    /// the `{data, successful, error, costUsd, …}` envelope the execute
    /// response wraps it in. So a downstream binding to one of these fields
    /// off a `tool_call` node must dereference `.item.json.data.<field>`
    /// (the engine's own `{json,text,raw}` envelope, THEN Composio's
    /// `data` wrapper), never the bare `.item.json.<field>` an agent/
    /// `http_request` output would use.
    pub output_fields: Vec<String>,
    /// The action's full output JSON Schema, when Composio publishes one.
    /// `None` means "unknown to this listing", not "empty" — mirrors
    /// [`composio_response_fields`]'s long-standing contract.
    pub output_schema: Option<Value>,
    /// Dotted path (relative to the envelope's own `json` field — prefix
    /// with `"json."` for a `split_out.path`, e.g. `"json.data.messages"`)
    /// to the first array-typed property in the tool's real runtime output,
    /// via [`compute_composio_array_path`]. Already accounts for Composio's
    /// `data` wrapper (see [`Self::output_fields`]'s doc) — this is NOT the
    /// bare [`compute_primary_array_path`] walk over [`Self::output_schema`],
    /// which is relative to the unwrapped payload and would be missing the
    /// leading `data.` segment. `None` when the output schema is unknown or
    /// names no array property.
    pub primary_array_path: Option<String>,
    /// Whether this action is ALSO one of OpenHuman's hand-curated actions
    /// for its toolkit (`catalog_for_toolkit` /
    /// `ComposioProvider::curated_tools`) — ranking signal only; a `false`
    /// here never hides a real action, it only sorts it after curated ones.
    pub is_curated: bool,
}

/// A [`LIVE_CATALOG_CACHE`] / [`PROBE_CACHE`] entry, timestamped so a lookup
/// can tell a fresh hit from an expired one (E-m8).
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    cached_at: std::time::Instant,
}

impl<T> CacheEntry<T> {
    fn fresh(value: T) -> Self {
        Self {
            value,
            cached_at: std::time::Instant::now(),
        }
    }

    /// `Some(&value)` while still within [`COMPOSIO_CATALOG_CACHE_TTL`],
    /// `None` once expired — callers treat an expired entry as a cache miss
    /// and re-fetch, same as no entry at all.
    fn if_fresh(&self) -> Option<&T> {
        (self.cached_at.elapsed() < COMPOSIO_CATALOG_CACHE_TTL).then_some(&self.value)
    }

    /// Backdates a fresh entry by `age` — test-only seam for exercising TTL
    /// expiry deterministically, without a mockable clock or a real 30-minute
    /// sleep (E-m8).
    #[cfg(test)]
    fn backdated_by(mut self, age: std::time::Duration) -> Self {
        self.cached_at -= age;
        self
    }
}

/// TTL for [`LIVE_CATALOG_CACHE`] and [`PROBE_CACHE`] entries (E-m8). Both
/// caches were previously permanent for the life of the process: a Composio
/// action published to a toolkit (or a corrected probe result) after the
/// first fetch stayed invisible — rejected by `flow_tool_allowed` Path B, or
/// serving a stale `primary_array_path`/`output_fields` — until the next
/// core restart. A support-visible gotcha, not a correctness bug: the
/// rejection direction is fail-CLOSED (an unknown/stale action is refused,
/// never silently permitted), so a bounded TTL only changes how long that
/// staleness can persist before self-healing, never which way an ambiguous
/// case resolves. 30 minutes comfortably outlasts a single builder session's
/// worth of repeat lookups (the reason these are caches at all) while
/// keeping "add a Composio action, come back later today" working without a
/// restart.
const COMPOSIO_CATALOG_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Process-level cache backing [`fetch_live_toolkit_catalog`]: lowercase
/// toolkit slug → every [`ToolContract`] the LIVE Composio catalog published
/// for it. One fetch per toolkit per TTL window — schemas are effectively
/// static within a session, but not forever (E-m8).
///
/// Replaces the narrower `REQUIRED_ARGS_CACHE` / `RESPONSE_FIELDS_CACHE`
/// pair (single-purpose, args-only / fields-only) that predated this fix:
/// [`composio_required_args`] and [`composio_response_fields`] now both
/// delegate to this one cache/fetch instead of each running its own
/// independent `composio_list_tools` round trip.
static LIVE_CATALOG_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, CacheEntry<Vec<ToolContract>>>>,
> = std::sync::OnceLock::new();

/// Per-toolkit miss coordination. The network fetch happens without holding
/// the cache's synchronous mutex; callers for the same key wait here, then
/// re-check the cache and reuse the first caller's result.
static LIVE_CATALOG_IN_FLIGHT: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::OnceLock::new();

fn live_catalog_fetch_lock(toolkit: &str) -> Option<std::sync::Arc<tokio::sync::Mutex<()>>> {
    let mut in_flight = LIVE_CATALOG_IN_FLIGHT
        .get_or_init(Default::default)
        .lock()
        .ok()?;
    Some(
        in_flight
            .entry(toolkit.to_string())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone(),
    )
}

/// Seeds the live-catalog cache for a toolkit — test hook so preflight /
/// search / contract-validation behavior can be exercised without a live
/// Composio backend. Replaces the narrower `seed_required_args_cache` /
/// `seed_response_fields_cache` test hooks this fix removes.
#[cfg(test)]
pub(crate) fn seed_live_catalog_cache(toolkit: &str, contracts: Vec<ToolContract>) {
    LIVE_CATALOG_CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("live catalog cache poisoned")
        .insert(
            toolkit.trim().to_ascii_lowercase(),
            CacheEntry::fresh(contracts),
        );
}

/// Like [`seed_live_catalog_cache`], but backdates the entry so it is
/// already expired — E-m8 TTL-expiry test seam.
#[cfg(test)]
pub(crate) fn seed_live_catalog_cache_expired(toolkit: &str, contracts: Vec<ToolContract>) {
    LIVE_CATALOG_CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("live catalog cache poisoned")
        .insert(
            toolkit.trim().to_ascii_lowercase(),
            CacheEntry::fresh(contracts)
                .backdated_by(COMPOSIO_CATALOG_CACHE_TTL + std::time::Duration::from_secs(1)),
        );
}

/// Fetches a toolkit's tool schemas STRAIGHT from the Composio client,
/// deliberately bypassing `composio::ops::composio_list_tools`'s curated-
/// whitelist filter (Direct mode's `filter_list_tools_response_for_direct` —
/// Backend mode's branch of `composio_list_tools` never filters at all, so
/// this is behavior-identical to it there) — so [`fetch_live_toolkit_catalog`]
/// grounds against the FULL live catalog (every real action, connected or
/// not, curated or not), not the narrower curated subset the pre-fix
/// `search_tool_catalog` searched.
///
/// - **Backend mode** calls [`crate::openhuman::integrations::composio::client::ComposioClient::list_tools`]
///   directly — already unfiltered (`composio_list_tools`'s backend branch
///   applies no filter either), so this is not a behavior change there.
/// - **Direct mode** calls [`direct_list_tools`] directly instead of going
///   through `composio_list_tools`'s direct branch, which DOES apply
///   `filter_list_tools_response_for_direct` — that's the filter this
///   function exists to skip. `direct_list_tools` itself never filters; the
///   curation is layered on entirely by its `composio_list_tools` caller.
///
/// Returns `None` on any client-construction or network failure — callers
/// degrade to "catalog unknown" rather than blocking.
async fn fetch_raw_toolkit_tools(
    config: &Config,
    toolkit: &str,
) -> Option<crate::openhuman::integrations::composio::types::ComposioToolsResponse> {
    let kind = create_composio_client(config)
        .map_err(|e| {
            tracing::debug!(target: "flows", %toolkit, error = %e, "[flows] live catalog: composio client unavailable — skipping");
            e
        })
        .ok()?;
    match kind {
        ComposioClientKind::Backend(client) => client
            .list_tools(Some(&[toolkit.to_string()]), None)
            .await
            .map_err(|e| {
                tracing::debug!(target: "flows", %toolkit, error = %e, "[flows] live catalog: backend fetch failed — skipping");
                e
            })
            .ok(),
        ComposioClientKind::Direct(tool) => direct_list_tools(&tool, &[toolkit.to_string()], None)
            .await
            .map_err(|e| {
                tracing::debug!(target: "flows", %toolkit, error = %e, "[flows] live catalog: direct fetch failed — skipping");
                e
            })
            .ok(),
    }
}

/// Fetches (or returns the cached) FULL LIVE Composio catalog for one
/// toolkit — every real action Composio publishes for it, mapped into
/// [`ToolContract`]s — regardless of OpenHuman's curated whitelist or the
/// user's connection state. This is the ground-truth source the Workflow
/// builder's discovery (`search_tool_catalog`/`get_tool_contract`) and
/// enforcement (`ops::validate_tool_contracts`) both consult.
///
/// Degrades gracefully when an action's listing carries no
/// `output_parameters` (unknown to this crate, or genuinely unpublished by
/// Composio for it) — `output_fields` is empty, `primary_array_path` is
/// `None`, and `output_schema` stays `None` so callers can distinguish "no
/// fields" from "schema unknown". Applies identically whether the listing
/// came from Direct mode (which threads `output_parameters` through
/// natively) or Backend mode (whatever its own proxy response carries under
/// the same field — may legitimately be absent).
///
/// `None` when the fetch itself failed (no client, network error) —
/// distinct from `Some(vec![])`, which means the toolkit is real but
/// currently publishes zero actions.
pub(crate) async fn fetch_live_toolkit_catalog(
    config: &Config,
    toolkit: &str,
) -> Option<Vec<ToolContract>> {
    use crate::openhuman::memory::sync::composio::providers::{
        catalog_for_toolkit, find_curated, get_provider,
    };

    let key = toolkit.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }

    if let Some(cached) = LIVE_CATALOG_CACHE
        .get_or_init(Default::default)
        .lock()
        .ok()?
        .get(&key)
        .and_then(CacheEntry::if_fresh)
    {
        return Some(cached.clone());
    }

    let fetch_lock = live_catalog_fetch_lock(&key)?;
    let _fetch_guard = fetch_lock.lock().await;

    // Another caller may have filled the cache while this one waited.
    if let Some(cached) = LIVE_CATALOG_CACHE
        .get_or_init(Default::default)
        .lock()
        .ok()?
        .get(&key)
        .and_then(CacheEntry::if_fresh)
    {
        return Some(cached.clone());
    }

    tracing::debug!(target: "flows", toolkit = %key, "[flows] live catalog: fetching (cache miss or expired)");
    let resp = fetch_raw_toolkit_tools(config, &key).await?;

    let curated_catalog = get_provider(&key)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&key));

    let contracts: Vec<ToolContract> = resp
        .tools
        .iter()
        .map(|tool| {
            let slug = tool.function.name.clone();
            let required_args = tool
                .function
                .parameters
                .as_ref()
                .and_then(|p| p.get("required"))
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let output_fields =
                response_fields_from_schema(tool.function.output_parameters.as_ref());
            let primary_array_path =
                compute_composio_array_path(tool.function.output_parameters.as_ref());
            let is_curated = curated_catalog.is_some_and(|cat| find_curated(cat, &slug).is_some());
            ToolContract {
                slug,
                toolkit: key.clone(),
                description: tool.function.description.clone(),
                required_args,
                input_schema: tool.function.parameters.clone(),
                output_fields,
                output_schema: tool.function.output_parameters.clone(),
                primary_array_path,
                is_curated,
            }
        })
        .collect();

    if let Ok(mut cache) = LIVE_CATALOG_CACHE.get_or_init(Default::default).lock() {
        cache.insert(key, CacheEntry::fresh(contracts.clone()));
    }
    Some(contracts)
}

#[cfg(test)]
mod in_flight_tests {
    use super::*;

    #[test]
    fn catalog_miss_locks_are_shared_per_toolkit_only() {
        let first = live_catalog_fetch_lock("test-lock-a").unwrap();
        let same = live_catalog_fetch_lock("test-lock-a").unwrap();
        let other = live_catalog_fetch_lock("test-lock-b").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &same));
        assert!(!std::sync::Arc::ptr_eq(&first, &other));
    }
}

/// [`compute_primary_array_path`], adjusted for the wrapper EVERY Composio
/// `tool_call` result carries at runtime.
///
/// A `tool_call` node's real output (the seam's tool invoker, which
/// `serde_json::to_value`s the client's `ComposioExecuteResponse` verbatim)
/// is `{data: <payload>, successful, error, costUsd, …}` — but the schema
/// Composio publishes as `output_parameters` (what [`compute_primary_array_path`]
/// walks) describes only `<payload>`, the content of that `data` field, not
/// the envelope around it. So the bare walk's result (e.g. `"messages"`) is
/// missing the `data.` segment a real `split_out.path`/downstream binding
/// needs (`"data.messages"`) — this wrapper adds it, UNCONDITIONALLY.
///
/// There is no escape hatch for a payload schema that itself happens to
/// declare a top-level `data` property (e.g. a provider whose real payload
/// shape is `{data: {messages: [...]}}`, unrelated to Composio's own
/// wrapper) — `output_parameters` describes the payload only, per the
/// invariant documented on [`ToolContract::output_fields`], so the real
/// runtime path in that case is `data.data.messages`, not `data.messages`.
/// Treating a payload-level `data` key as "this schema already models the
/// envelope" silently drops a real wrapper segment and points a downstream
/// binding / `split_out.path` at the wrong (non-existent) array.
pub(crate) fn compute_composio_array_path(schema: Option<&Value>) -> Option<String> {
    let path = compute_primary_array_path(schema)?;
    Some(format!("data.{path}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Real-output probe (systemic tool-contract fix, Part 3 / B12)
// ─────────────────────────────────────────────────────────────────────────────
//
// [`compute_composio_array_path`] above is entirely schema-derived — it has
// nothing to walk when Composio (or the backend-proxied listing path, see
// [`crate::openhuman::integrations::composio::ComposioToolFunction::output_parameters`]'s
// doc) simply never publishes `output_parameters` for an action. Verified
// live: EVERY GitHub action's `get_tool_contract` (including the curated
// `GITHUB_LIST_REPOSITORY_ISSUES`) comes back `output_fields: [],
// output_schema: null, primary_array_path: null` — there is no schema at all
// to fix a walker bug in. The one remaining source of ground truth is the
// real response itself, so [`probe_tool_output_sample`] makes ONE bounded,
// READ-only, REAL Composio call and derives the same shape of hint
// (`primary_array_path`/`output_fields`) from the ACTUAL value instead.
//
// This is the exact bug observed live on flow "funny reminders v2": with no
// schema to consult, the builder guessed `split_out.path = "json.data"` (the
// whole envelope payload — one item, the `{issues:[...]}` container) instead
// of the real `"json.data.issues"`, and the downstream condition/agent saw
// the wrong shape and produced zero reminders.

/// Top-level [`crate::openhuman::integrations::composio::ComposioExecuteResponse`] fields
/// that are never part of the tool's own payload — skipped at the ROOT by
/// [`compute_primary_array_path_from_value`]'s scan so an envelope field
/// never masquerades as (or shadows) the real array. None of these are ever
/// arrays in practice, but the skip is explicit so a future envelope field
/// can't silently win a shallowest-wins tie against a real nested array.
/// `pub(crate)` because the workflow adapter seam passes this into the
/// vendor-neutral walker in [`crate::openhuman::json_schema`]. That walker
/// deliberately takes the skip-list as a parameter rather than knowing any
/// provider's envelope shape, so this constant is the piece of Composio
/// knowledge the caller supplies — exporting it is the seam, not a leak.
pub(crate) const COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT: &[&str] =
    &["successful", "error", "costUsd", "markdownFormatted"];

/// One real, LIVE-sampled Composio action result — [`probe_tool_output_sample`]'s
/// cached ground truth, keyed by action slug (uppercased). Distinct from
/// (and takes priority over, via [`apply_probe_override`]) the schema-derived
/// fields on [`ToolContract`]: a probe is an ACTUAL observed response, not a
/// published schema Composio may or may not provide.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ProbedOutputSample {
    /// Dotted path (relative to the envelope's own `json` field — prefix with
    /// `"json."` for a `split_out.path`, same convention as
    /// [`ToolContract::primary_array_path`]) to the first array found in the
    /// real response. `None` when the real response named no array at all.
    pub primary_array_path: Option<String>,
    /// Top-level field names of the real response's `data` payload — the
    /// probed analogue of [`ToolContract::output_fields`].
    pub output_fields: Vec<String>,
    /// The full envelope-shaped sample value the probe observed, verbatim —
    /// returned to `probe_tool_output_sample`'s IMMEDIATE caller only for
    /// this one call. **Never persisted** into [`PROBE_CACHE`]
    /// ([`cache_probe_result`] redacts it to `Value::Null` before inserting)
    /// — the process-wide cache is keyed by slug alone, and a real probe
    /// response can carry one user/connection/args' actual private data
    /// (repo issues, messages, …); nothing else in the process reads a
    /// CACHED sample (only the derived `primary_array_path`/`output_fields`
    /// do), so retaining the full payload there would be pure unnecessary
    /// exposure (see PR #4702 review).
    pub sample: Value,
}

/// Process-level cache backing [`probe_tool_output_sample`]: action slug
/// (uppercased) → the [`ProbedOutputSample`] it produced. One real probe per
/// slug per TTL window (E-m8) — mirrors [`LIVE_CATALOG_CACHE`]'s shape, and
/// for the same reason: a probe is a real, potentially rate-limited/billed
/// external call, not something to repeat every turn.
///
/// Entries here always have `sample` redacted to `Value::Null` — see
/// [`cache_probe_result`] and [`ProbedOutputSample::sample`]'s doc.
static PROBE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, CacheEntry<ProbedOutputSample>>>,
> = std::sync::OnceLock::new();

/// Seeds the probe cache for a slug — test hook so [`apply_probe_override`]
/// and the enforcement checks that consult a probe can be exercised without a
/// live Composio backend. Unlike [`cache_probe_result`], does NOT redact
/// `sample` — tests seed only small synthetic fixtures, never real user data.
#[cfg(test)]
pub(crate) fn seed_probe_cache(slug: &str, sample: ProbedOutputSample) {
    PROBE_CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("probe cache poisoned")
        .insert(slug.trim().to_ascii_uppercase(), CacheEntry::fresh(sample));
}

/// Like [`seed_probe_cache`], but backdates the entry so it is already
/// expired — E-m8 TTL-expiry test seam.
#[cfg(test)]
pub(crate) fn seed_probe_cache_expired(slug: &str, sample: ProbedOutputSample) {
    PROBE_CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("probe cache poisoned")
        .insert(
            slug.trim().to_ascii_uppercase(),
            CacheEntry::fresh(sample)
                .backdated_by(COMPOSIO_CATALOG_CACHE_TTL + std::time::Duration::from_secs(1)),
        );
}

/// Caches the DERIVED metadata from a real probe — never the raw `sample`
/// payload itself (redacted to `Value::Null` here). See
/// [`ProbedOutputSample::sample`]'s doc for why: a real probe response can
/// contain one user/connection/args' actual private data, and nothing that
/// reads from the cache (only [`apply_probe_override`]) ever needs the raw
/// payload — only the derived `primary_array_path`/`output_fields`.
fn cache_probe_result(slug: &str, sample: ProbedOutputSample) {
    let cached = ProbedOutputSample {
        sample: Value::Null,
        ..sample
    };
    if let Ok(mut cache) = PROBE_CACHE.get_or_init(Default::default).lock() {
        cache.insert(slug.trim().to_ascii_uppercase(), CacheEntry::fresh(cached));
    }
}

/// Looks up a cached [`ProbedOutputSample`] for `slug`, or `None` when
/// [`probe_tool_output_sample`] has never successfully probed it within the
/// current TTL window (E-m8) — an expired probe is treated exactly like a
/// process that never probed it at all, and re-probes on next use.
pub(crate) fn probed_output_sample(slug: &str) -> Option<ProbedOutputSample> {
    PROBE_CACHE
        .get_or_init(Default::default)
        .lock()
        .ok()?
        .get(&slug.trim().to_ascii_uppercase())
        .and_then(CacheEntry::if_fresh)
        .cloned()
}

/// Overlays a cached [`probed_output_sample`] (if any) onto a schema-derived
/// [`ToolContract`] — the probe, being an ACTUAL observed response, always
/// wins over the schema-derived hint when both are present. A contract with
/// no cached probe passes through unchanged. Called everywhere a
/// [`ToolContract`] is consulted for wiring (`get_tool_contract`,
/// `graph_output_field_warnings`, `graph_split_out_path_warnings`) so a
/// probe the builder already ran is never shadowed by a stale/absent schema.
///
/// `primary_array_path` is overlaid UNCONDITIONALLY (including `None`) once a
/// probe exists: a probe's `None` is itself meaningful ("the real response
/// named no array anywhere"), not "no opinion" — leaving a stale
/// schema-derived path in place after a real observation disproves it would
/// let a since-confirmed-wrong `split_out.path` keep looking supported (see
/// PR #4702 review). `output_fields` only overlays when non-empty since an
/// empty probe result there genuinely means "unknown", not "confirmed empty".
pub(crate) fn apply_probe_override(mut contract: ToolContract) -> ToolContract {
    if let Some(probe) = probed_output_sample(&contract.slug) {
        contract.primary_array_path = probe.primary_array_path;
        if !probe.output_fields.is_empty() {
            contract.output_fields = probe.output_fields;
        }
    }
    contract
}

/// Best-effort, but FAIL-CLOSED, classification of a Composio action slug's
/// [`ToolScope`] — mirrors `flow_tool_allowed`'s Path A / Path B split
/// rather than trusting `classify_unknown`'s verb heuristic unconditionally:
///
/// - Toolkit has no extractable prefix at all (`toolkit_from_slug` fails):
///   `None` — nothing to confirm a scope against.
/// - Toolkit HAS a static curated catalog (`get_provider().curated_tools()`
///   or `catalog_for_toolkit`): the slug's scope is authoritative ONLY if the
///   slug is actually one of that catalog's curated entries. An uncurated
///   slug on a cataloged toolkit resolves to `None` — it must NOT fall
///   through to the verb heuristic, which can misclassify an uncurated write
///   action (e.g. a connected GitHub/Gmail action not in the curated list)
///   as `Read` by name alone (see PR #4702 review — this exact hole would
///   otherwise let the probe execute a real write).
/// - Toolkit has NO static catalog at all: falls back to `classify_unknown`
///   — the same authority `flow_tool_allowed`'s Path B accepts once it has
///   already confirmed (via its own connected + live-catalog checks) the
///   slug is real; here it's just the scope signal, gated further below.
///
/// Used exclusively by [`probe_tool_output_sample`] to hard-refuse anything
/// that isn't a CONFIDENTLY CONFIRMED `Read` action REGARDLESS of the user's
/// per-toolkit scope preference — unlike `flow_tool_allowed`, which honors
/// a user's opt-in to Write/Admin for a real `tool_call` node, a
/// schema-discovery probe must never perform a real mutation no matter what
/// the user has toggled on, and must never rely on a heuristic guess to
/// decide that: the builder never asked for (and the user never approved)
/// THIS specific write. `None` means "refuse — no confirmed Read scope", not
/// "assume Read".
fn resolve_composio_action_scope(
    slug: &str,
) -> Option<crate::openhuman::memory::sync::composio::providers::ToolScope> {
    use crate::openhuman::memory::sync::composio::providers::{
        catalog_for_toolkit, classify_unknown, find_curated, get_provider, toolkit_from_slug,
    };

    let toolkit = toolkit_from_slug(slug)?;
    match get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit))
    {
        // A static catalog exists for this toolkit — only a genuinely
        // curated entry's scope is trustworthy; an uncurated slug fails
        // closed rather than being guessed via the verb heuristic.
        Some(catalog) => find_curated(catalog, slug).map(|curated| curated.scope),
        // No static catalog anywhere for this toolkit — the heuristic is
        // the only available signal (still gated by the connected-toolkit
        // check below in `probe_tool_output_sample`).
        None => Some(classify_unknown(slug)),
    }
}

/// `get_tool_output_sample`'s implementation — see the module comment above
/// this section for why it exists. Gates, in order (fail closed on any):
///
/// 1. **Scope**: [`resolve_composio_action_scope`] must CONFIRM `slug` as
///    `Read` (`None` — no confirmed scope, e.g. an uncurated slug on a
///    cataloged toolkit — refuses exactly like a confirmed non-`Read` scope
///    does; it is never treated as "assume Read").
/// 2. **Connected**: the slug's toolkit must have an active Composio
///    connection.
///
/// On success, derives + caches a [`ProbedOutputSample`] (process-lifetime,
/// keyed by slug) and returns it. `args` is forwarded verbatim to the real
/// call — the builder should pass the SAME arguments it intends to wire into
/// the real `tool_call` node (this is a sample of THAT call, not a generic
/// fixture); omitted/`null` becomes `{}`, which is fine for a
/// zero-required-arg action.
pub(crate) async fn probe_tool_output_sample(
    config: &Config,
    slug: &str,
    args: Value,
) -> std::result::Result<ProbedOutputSample, String> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err("get_tool_output_sample: slug must not be empty".to_string());
    }

    match resolve_composio_action_scope(slug) {
        Some(crate::openhuman::memory::sync::composio::providers::ToolScope::Read) => {}
        Some(other) => {
            tracing::warn!(
                target: "flows",
                %slug,
                scope = other.as_str(),
                "[flows] get_tool_output_sample: refused — not a Read-scope action"
            );
            return Err(format!(
                "get_tool_output_sample refuses `{slug}`: classified as {} — this probe is \
                 READ-only and never performs a real mutation, regardless of the user's scope \
                 preference. Use get_tool_contract for its schema-derived (possibly unknown) \
                 output shape instead.",
                other.as_str()
            ));
        }
        None => {
            tracing::warn!(
                target: "flows",
                %slug,
                "[flows] get_tool_output_sample: refused — no confirmed Read scope (either no \
                 extractable toolkit, or an uncurated slug on a toolkit with a static curated \
                 catalog — fails closed rather than guessing via the verb heuristic)"
            );
            return Err(format!(
                "get_tool_output_sample refuses `{slug}`: could not confirm this is a Read-scope \
                 action. Either no toolkit could be extracted from the slug, or its toolkit ships \
                 a static curated catalog and this slug is not one of its curated actions — this \
                 probe never falls back to a verb-name heuristic in that case, since an uncurated \
                 action on a cataloged toolkit could really be a write. Use get_tool_contract for \
                 its schema-derived (possibly unknown) output shape instead."
            ));
        }
    }

    let Some(toolkit) =
        crate::openhuman::memory::sync::composio::providers::toolkit_from_slug(slug)
    else {
        return Err(format!(
            "get_tool_output_sample: could not extract a toolkit from slug '{slug}' — it must \
             look like '<TOOLKIT>_<ACTION>'."
        ));
    };

    let integrations =
        crate::openhuman::integrations::composio::fetch_connected_integrations(config).await;
    let connected = integrations
        .iter()
        .any(|i| i.connected && i.toolkit.eq_ignore_ascii_case(&toolkit));
    if !connected {
        tracing::warn!(target: "flows", %slug, %toolkit, "[flows] get_tool_output_sample: refused — toolkit not connected");
        return Err(format!(
            "get_tool_output_sample refuses `{slug}`: the '{toolkit}' toolkit has no active \
             Composio connection for this user — connect it first (composio_connect), or fall \
             back to get_tool_contract's schema-derived hint."
        ));
    }

    tracing::debug!(
        target: "flows",
        %slug,
        %toolkit,
        "[flows] get_tool_output_sample: probing the real live response (read-only, bounded, one call)"
    );

    let kind = create_composio_client(config).map_err(|e| e.to_string())?;
    let args_opt = if args.is_null() { None } else { Some(args) };
    let resp = match kind {
        ComposioClientKind::Backend(client) => client
            .execute_tool(slug, args_opt)
            .await
            .map_err(|e| format!("get_tool_output_sample: real call to `{slug}` failed: {e}"))?,
        ComposioClientKind::Direct(tool) => {
            direct_execute(&tool, slug, args_opt, &config.composio.entity_id, None)
                .await
                .map_err(|e| format!("get_tool_output_sample: real call to `{slug}` failed: {e}"))?
        }
    };

    if !resp.successful {
        let detail = resp
            .error
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or("no error detail returned by the provider");
        return Err(format!(
            "get_tool_output_sample: `{slug}` reported failure at the connected provider: {detail}"
        ));
    }

    let envelope = serde_json::to_value(&resp).map_err(|e| {
        format!("get_tool_output_sample: could not serialize the real response: {e}")
    })?;
    let primary_array_path =
        compute_primary_array_path_from_value(&envelope, COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT);
    let output_fields = resp
        .data
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    let sample = ProbedOutputSample {
        primary_array_path,
        output_fields,
        sample: envelope,
    };
    cache_probe_result(slug, sample.clone());
    tracing::info!(
        target: "flows",
        %slug,
        primary_array_path = ?sample.primary_array_path,
        "[flows] get_tool_output_sample: probed + cached the real output shape"
    );
    Ok(sample)
}

/// Best-effort lookup of a Composio action's **required** top-level parameter
/// names — a thin projection over [`fetch_live_toolkit_catalog`]'s
/// [`ToolContract`]s (this used to run its own independent
/// `REQUIRED_ARGS_CACHE`-backed fetch; existing callers — the required-arg
/// preflight, `graph_wiring_warnings` — keep this exact signature).
///
/// Returns `None` when the schema is unavailable — unknown toolkit, client
/// construction failure, a failed/empty listing, or the slug isn't present
/// in the toolkit's live catalog — so callers can skip the preflight rather
/// than block execution on a catalog hiccup.
pub(crate) async fn composio_required_args(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = crate::openhuman::memory::sync::composio::providers::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))
        .map(|c| c.required_args.clone())
}

/// Best-effort lookup of a Composio action's **response/output** top-level
/// field names — the output-side analogue of [`composio_required_args`],
/// now a thin projection over [`fetch_live_toolkit_catalog`]'s
/// [`ToolContract`]s (replaces the standalone `RESPONSE_FIELDS_CACHE`-backed
/// fetch; `search_tool_catalog`'s grounding keeps this exact signature).
///
/// Returns `None` when no output schema is known for the slug — unknown
/// toolkit, client construction failure, a failed/empty listing, the slug
/// isn't in the live catalog, or a real action whose listing doesn't
/// publish `output_parameters` — so callers degrade to "output shape
/// unknown" rather than blocking or guessing. `Some(vec![])` means the
/// schema was found but names no top-level properties.
pub(crate) async fn composio_response_fields(config: &Config, slug: &str) -> Option<Vec<String>> {
    let toolkit = crate::openhuman::memory::sync::composio::providers::toolkit_from_slug(slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    let contract = contracts
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug))?;
    contract.output_schema.as_ref()?;
    Some(contract.output_fields.clone())
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
