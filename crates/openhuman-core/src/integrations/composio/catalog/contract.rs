//! The LIVE, ground-truth Composio tool contract: [`ToolContract`] itself
//! and the per-toolkit cache/fetch ([`fetch_live_toolkit_catalog`]) that
//! backs it — sourced straight from Composio's own v3 `/tools` listing.
//! See the parent module's docs for why this lives in `composio` at all.

use serde_json::Value;

use crate::config::Config;
use crate::integrations::composio::client::{
    direct_list_tools, resolve_composio_route, ComposioRoute,
};
use crate::integrations::composio::module_client::{self as connectors};
use crate::integrations::composio::types::{ComposioListToolsRequest, ComposioToolsResponse};
use tinyagents_harness::tool::{compute_primary_array_path, response_fields_from_schema};
use tinyconnectors_bus::names::methods;

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
pub(super) struct CacheEntry<T> {
    value: T,
    cached_at: std::time::Instant,
}

impl<T> CacheEntry<T> {
    pub(super) fn fresh(value: T) -> Self {
        Self {
            value,
            cached_at: std::time::Instant::now(),
        }
    }

    /// `Some(&value)` while still within [`COMPOSIO_CATALOG_CACHE_TTL`],
    /// `None` once expired — callers treat an expired entry as a cache miss
    /// and re-fetch, same as no entry at all.
    pub(super) fn if_fresh(&self) -> Option<&T> {
        (self.cached_at.elapsed() < COMPOSIO_CATALOG_CACHE_TTL).then_some(&self.value)
    }

    /// Backdates a fresh entry by `age` — test-only seam for exercising TTL
    /// expiry deterministically, without a mockable clock or a real 30-minute
    /// sleep (E-m8).
    #[cfg(test)]
    pub(super) fn backdated_by(mut self, age: std::time::Duration) -> Self {
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
pub(super) const COMPOSIO_CATALOG_CACHE_TTL: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

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

pub(super) fn live_catalog_fetch_lock(
    toolkit: &str,
) -> Option<std::sync::Arc<tokio::sync::Mutex<()>>> {
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

/// Fetches a toolkit's tool schemas STRAIGHT from the connector module (or, in
/// direct mode, the host-side v3 reader),
/// deliberately bypassing `composio::ops::composio_list_tools`'s curated-
/// whitelist filter (Direct mode's `filter_list_tools_response_for_direct` —
/// Backend mode's branch of `composio_list_tools` never filters at all, so
/// this is behavior-identical to it there) — so [`fetch_live_toolkit_catalog`]
/// grounds against the FULL live catalog (every real action, connected or
/// not, curated or not), not the narrower curated subset the pre-fix
/// `search_tool_catalog` searched.
///
/// - **Backend mode** calls the module's `ListTools` member
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
async fn fetch_raw_toolkit_tools(config: &Config, toolkit: &str) -> Option<ComposioToolsResponse> {
    let kind = resolve_composio_route(config)
        .map_err(|e| {
            tracing::debug!(target: "flows", %toolkit, error = %e, "[flows] live catalog: composio client unavailable — skipping");
            e
        })
        .ok()?;
    match kind {
        ComposioRoute::Backend => connectors::call::<_, ComposioToolsResponse>(
            config,
            methods::LIST_TOOLS,
            ComposioListToolsRequest {
                toolkits: vec![toolkit.to_string()],
                tags: Vec::new(),
                apply_user_scopes: false,
            },
        )
        .await
            .map_err(|e| {
                tracing::debug!(target: "flows", %toolkit, error = %e, "[flows] live catalog: backend fetch failed — skipping");
                e
            })
            .ok(),
        ComposioRoute::Direct(tool) => direct_list_tools(config, &tool, &[toolkit.to_string()], None)
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
    use crate::integrations::composio::contract::{catalog_for_toolkit, find_curated};

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

    let curated_catalog = catalog_for_toolkit(&key);

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
