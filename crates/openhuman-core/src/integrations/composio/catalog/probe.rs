//! Real-output probe (systemic tool-contract fix, Part 3 / B12): the
//! caches, gates, and helper that back [`probe_tool_output_sample`] and
//! [`apply_probe_override`] — the ACTUAL-observed-response counterpart to
//! [`super::contract::fetch_live_toolkit_catalog`]'s schema-derived
//! [`super::contract::ToolContract`].

use serde_json::Value;

#[cfg(test)]
use super::contract::COMPOSIO_CATALOG_CACHE_TTL;
use super::contract::{CacheEntry, ToolContract};
use crate::config::Config;
use tinyagents_harness::tool::compute_primary_array_path_from_value;

// ─────────────────────────────────────────────────────────────────────────────
// Real-output probe (systemic tool-contract fix, Part 3 / B12)
// ─────────────────────────────────────────────────────────────────────────────
//
// [`compute_composio_array_path`] above is entirely schema-derived — it has
// nothing to walk when Composio (or the backend-proxied listing path, see
// [`crate::integrations::composio::ComposioToolFunction::output_parameters`]'s
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

/// Top-level [`crate::integrations::composio::ComposioExecuteResponse`] fields
/// that are never part of the tool's own payload — skipped at the ROOT by
/// [`compute_primary_array_path_from_value`]'s scan so an envelope field
/// never masquerades as (or shadows) the real array. None of these are ever
/// arrays in practice, but the skip is explicit so a future envelope field
/// can't silently win a shallowest-wins tie against a real nested array.
/// `pub(crate)` because the workflow adapter seam passes this into the
/// vendor-neutral walker in `tinyagents_harness::tool::schema_walk`. That walker
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
pub(super) fn cache_probe_result(slug: &str, sample: ProbedOutputSample) {
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
pub(super) fn resolve_composio_action_scope(
    slug: &str,
) -> Option<crate::integrations::composio::contract::ToolScope> {
    use crate::integrations::composio::contract::{
        catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug,
    };

    let toolkit = toolkit_from_slug(slug)?;
    match catalog_for_toolkit(&toolkit) {
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
        Some(crate::integrations::composio::contract::ToolScope::Read) => {}
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

    let Some(toolkit) = crate::integrations::composio::contract::toolkit_from_slug(slug) else {
        return Err(format!(
            "get_tool_output_sample: could not extract a toolkit from slug '{slug}' — it must \
             look like '<TOOLKIT>_<ACTION>'."
        ));
    };

    let integrations = crate::integrations::composio::fetch_connected_integrations(config).await;
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

    let args_opt = if args.is_null() { None } else { Some(args) };
    let resp = crate::integrations::composio::execute_dispatch::execute_composio_action(
        config, slug, args_opt, None,
    )
    .await
    .map_err(|e| format!("get_tool_output_sample: real call to `{slug}` failed: {e}"))?;

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
