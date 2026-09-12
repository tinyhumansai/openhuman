//! The capability seam: five adapters implementing `tinyflows::caps` traits
//! over real OpenHuman services.
//!
//! Each tinyflows integration node hands its **whole** `node.config` to the
//! matching trait method — the adapter interprets a free-form JSON value the
//! flow author wrote, pulling a connection ref out of `config["connection_ref"]`
//! where relevant. See `my_docs/ohxtf/b1-engine-seam-domain/04-capability-seam.md`
//! for the source-verified node → trait contract this mirrors.
//!
//! All host errors are mapped to `tinyflows::error::EngineError::Capability`,
//! per the crate's contract (`caps` traits return `tinyflows::error::Result`).

use std::sync::Arc;

use crate::openhuman::flows::tinyflows::checkpoint_sqlite::SqliteCheckpointer;
use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};
#[cfg(test)]
use tinyflows::model::WorkflowGraph;

use crate::openhuman::config::Config;
#[cfg(test)]
use crate::openhuman::config::HttpRequestConfig;
#[cfg(test)]
use crate::openhuman::flows;
#[cfg(test)]
use crate::openhuman::security::credentials::HttpCredential;
use crate::openhuman::security::credentials::HttpCredentialsStore;
use crate::openhuman::security::{CommandClass, SecurityPolicy};
#[cfg(test)]
use crate::openhuman::security::{GateDecision, POLICY_BLOCKED_MARKER};

// The JSON Schema walkers moved to `openhuman::json_schema`, a domain owned by
// neither this seam nor `composio` — see that module's docs for why neutral
// ownership is load-bearing rather than tidiness.
//
// Re-exported so `crate::openhuman::flows::tinyflows::caps::<fn>` keeps resolving for
// the callers outside this module (`flows::ops`, `tinyflows::tests`) — the
// relocation is an internal reorganization, not an API change.
// The live Composio catalog and probe moved to `composio::catalog` -- the domain
// that owns Composio's vocabulary. This import is the edge pointing the right
// way round: the feature-gated seam depends on the always-compiled domain, not
// the reverse. See that module's docs.
#[cfg(test)]
pub(crate) use crate::openhuman::integrations::composio::catalog::ProbedOutputSample;
pub(crate) use crate::openhuman::integrations::composio::catalog::{
    apply_probe_override, composio_required_args, fetch_live_toolkit_catalog,
    probe_tool_output_sample, ToolContract,
};
#[cfg(test)]
pub(crate) use crate::openhuman::integrations::composio::catalog::{
    seed_live_catalog_cache, seed_live_catalog_cache_expired, seed_probe_cache,
};

use super::*;

#[cfg(test)]
pub(crate) use crate::openhuman::json_schema::{
    compute_primary_array_path, response_fields_from_schema,
};
pub(crate) use crate::openhuman::json_schema::{missing_required_args, unsupported_arg_names};

/// Parses a `"composio:<toolkit>:<connection_id>"` `connection_ref` (see the
/// node catalog, `my_docs/ohxtf/commons/12-node-catalog-0.2.md`) and returns
/// the trailing connection id segment. Values that don't match this shape
/// return `None` — the caller logs and falls back to the ambient session
/// account (only Direct mode can actually forward the id today; see
/// [`OpenHumanTools::invoke`]'s doc for the Backend-mode gap this leaves
/// open).
pub(crate) fn composio_connection_id(conn: &str) -> Option<&str> {
    let rest = conn.strip_prefix("composio:")?;
    let id = rest.rsplit(':').next()?;
    (!id.is_empty()).then_some(id)
}

/// Parses a `"http_cred:<name>"` `connection_ref` for [`OpenHumanHttp`],
/// returning the trailing credential name. The host-side
/// [`HttpCredentialsStore`] (encrypted-at-rest bearer/basic/header
/// templates) is real and load-bearing — [`resolve_http_credential`] looks
/// the extracted name up in it and injects the resolved auth header
/// server-side. This function only does the parse; a malformed or missing
/// name (`None`) is what lets the caller fail the request closed instead of
/// silently sending it unauthenticated. See [`OpenHumanHttp::request`]'s doc
/// and the "Phase 2" note on the [`OpenHumanHttp`] struct for the full
/// resolution flow.
pub(crate) fn http_cred_name(conn: &str) -> Option<&str> {
    let name = conn.strip_prefix("http_cred:")?.trim();
    (!name.is_empty()).then_some(name)
}

/// Strict, deny-by-default curation check for flow `tool_call` nodes (issue
/// B2 finding #2).
///
/// This is intentionally **stricter** than
/// `memory_sync::composio::providers::is_action_visible_with_pref` — the
/// helper the normal agent tool-call loop uses. That helper is permissive by
/// design for a toolkit it doesn't recognize: it falls back to the
/// `classify_unknown` heuristic and lets the slug through (scope-gated), and
/// treats a prefix-less slug as unconditionally visible. That's safe in the
/// agent loop because the model only ever sees slugs the *backend itself*
/// returned from live tool discovery (`composio_list_tools`) — there is no
/// path for the model to invent a slug that reaches this check. A flow's
/// `tool_call.slug`, by contrast, is a free-form string the flow *author*
/// typed when building the graph; it never round-trips through Composio
/// discovery before `invoke` is called. So here a slug is allowed **only**
/// if it resolves to a real, known toolkit AND is present in that toolkit's
/// curated catalog:
/// - `toolkit_from_slug` fails to extract anything (empty/blank slug) → reject.
/// - the extracted toolkit has no registered provider curated list AND no
///   static `catalog_for_toolkit` entry (i.e. it isn't one of OpenHuman's
///   known/curated toolkits at all — including a made-up prefix like
///   `madeupkit`, or a prefix-less slug like `noop` which `toolkit_from_slug`
///   degrades to treating as its own single-segment "toolkit") → reject.
/// - the toolkit has a catalog but `slug` isn't one of its entries → reject.
/// - otherwise, apply the same per-user read/write/admin scope preference
///   the agent loop uses (`UserScopePref::allows`).
///
/// // (0.3) The former hard-reject of any *real* Composio toolkit not in the
/// // static `catalog_for_toolkit` map is now lifted for toolkits the user has
/// // actually connected: when a slug's toolkit has no static curated catalog,
/// // the gate consults the user's **live connected-toolkit set** (from the
/// // composio domain) and allows the call iff the user holds an ACTIVE
/// // connection for that toolkit. A genuinely-unknown/made-up toolkit is never
/// // connected, so it still rejects. Toolkits OpenHuman *does* ship a static
/// // catalog for keep their stricter curated-action + per-user scope gating
/// // unchanged (a connected-but-uncurated action on a cataloged toolkit is
/// // still rejected — the catalog is the tighter allowlist there).
///
/// // (systemic tool-contract fix, PR2) Path B is now further tightened rather
/// // than loosened: on top of the (0.3) connected-toolkit check, the SLUG
/// // ITSELF must be a genuine action in that toolkit's LIVE Composio catalog
/// // (`fetch_live_toolkit_catalog`) — previously any string sharing the
/// // connected toolkit's prefix passed (e.g. a hallucinated/typo'd
/// // `STRIPE_DOES_NOT_EXIST` for a connected `stripe`), with no per-user
/// // read/write/admin scope check at all. Now: existence is broadened to the
/// // real catalog (a real-but-uncurated action is allowed), but scope gating
/// // is ADDED via [`classify_unknown`] — strictly narrower than before, never
/// // looser.
///
/// Returns whether `slug` may be invoked as a flow `tool_call`, given (only when
/// needed) the user's live connected-toolkit slug set. `config` is only used by
/// Path B's live-catalog fetch (fed through [`fetch_live_toolkit_catalog`],
/// which is itself cached — a seeded test cache never touches the network).
///
/// Split out from [`is_curated_flow_tool`] as a (mostly) pure function so the
/// two decision paths are unit-testable without a live Composio backend:
/// `connected_toolkits` is `None` when the toolkit has a static catalog (the
/// connected set is never consulted then) or when the connected set could not
/// be fetched (fail-closed).
async fn flow_tool_allowed(
    config: &Config,
    slug: &str,
    connected_toolkits: Option<&[String]>,
) -> bool {
    use crate::openhuman::integrations::composio::ops::load_user_scope_pref;
    use crate::openhuman::integrations::composio::providers::{
        catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug,
    };

    let Some(toolkit) = toolkit_from_slug(slug) else {
        tracing::debug!(target: "flows", %slug, "[flows] tool_call curation: reject — slug has no extractable toolkit prefix");
        return false;
    };

    // Path A: a toolkit OpenHuman ships a static curated catalog for keeps its
    // strict curated-action + per-user scope gating (unchanged from B2).
    if let Some(catalog) = catalog_for_toolkit(&toolkit) {
        let Some(curated) = find_curated(catalog, slug) else {
            tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — slug is not a curated action of this toolkit");
            return false;
        };
        let pref = load_user_scope_pref(config, &toolkit).await;
        let allowed = pref.allows(curated.scope);
        tracing::debug!(target: "flows", %slug, %toolkit, allowed, "[flows] tool_call curation: static curated catalog decision");
        return allowed;
    }

    // Path B: no static catalog. First, the (0.3) toolkit-level gate — allow
    // only when the user has a live ACTIVE Composio connection for it. A
    // made-up toolkit is never connected, so it rejects right here without
    // ever reaching the live-catalog fetch below.
    let connected = match connected_toolkits {
        Some(toolkits) => toolkits.iter().any(|t| t.eq_ignore_ascii_case(&toolkit)),
        None => {
            tracing::warn!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — no static catalog and the connected-toolkit set was unavailable (fail-closed)");
            false
        }
    };
    if !connected {
        tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — toolkit has no static catalog and is not connected");
        return false;
    }

    // Second, the (systemic tool-contract fix) slug-existence gate — the
    // exact slug must be a genuine action in the toolkit's LIVE Composio
    // catalog, not merely share its prefix. A fetch failure fails closed
    // (never falls back to "any slug with the right prefix passes").
    let Some(live_catalog) = fetch_live_toolkit_catalog(config, &toolkit).await else {
        tracing::warn!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — connected but the live catalog fetch failed (fail-closed)");
        return false;
    };
    if !live_catalog
        .iter()
        .any(|c| c.slug.eq_ignore_ascii_case(slug))
    {
        tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — slug is not a real action in this toolkit's live catalog");
        return false;
    }

    // Finally, scope-gate the same way a curated action is — via the
    // classify_unknown heuristic (mirrors
    // `providers::is_action_visible_with_pref`'s uncurated branch), which the
    // pre-fix Path B never applied at all.
    let pref = load_user_scope_pref(config, &toolkit).await;
    let allowed = pref.allows(classify_unknown(slug));
    tracing::debug!(target: "flows", %slug, %toolkit, allowed, "[flows] tool_call curation: live catalog + scope decision");
    allowed
}

/// Whether `slug`'s toolkit lacks a static curated catalog, i.e. the curation
/// decision must consult the user's live connected-toolkit set. Kept cheap and
/// offline (a registry lookup) so the common cataloged-toolkit path never pays
/// for a connected-set fetch.
fn slug_needs_connected_set(slug: &str) -> bool {
    use crate::openhuman::integrations::composio::providers::{
        catalog_for_toolkit, toolkit_from_slug,
    };
    match toolkit_from_slug(slug) {
        Some(toolkit) => catalog_for_toolkit(&toolkit).is_none(),
        None => false,
    }
}

/// The user's live set of ACTIVE-connected Composio toolkit slugs (lowercased),
/// or `None` when the backend is unreachable and no cached snapshot exists.
///
/// Uses [`fetch_connected_integrations_status`] so a transient backend failure
/// (`Unavailable`) is distinguished from "confirmed zero connections" — on
/// `Unavailable` we fall back to the last-known (even expired) cache rather than
/// collapse the allowlist to empty, and only return `None` when there is truly
/// nothing to go on (the caller then fails closed).
async fn connected_toolkit_slugs(config: &Config) -> Option<Vec<String>> {
    use crate::openhuman::integrations::composio::{
        cached_active_integrations_including_expired, fetch_connected_integrations_status,
        FetchConnectedIntegrationsStatus,
    };

    let integrations = match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(v) => v,
        FetchConnectedIntegrationsStatus::Unavailable => {
            match cached_active_integrations_including_expired(config) {
                Some(v) => {
                    tracing::warn!(target: "flows", "[flows] connected-toolkit lookup: backend unavailable — using last-known (possibly stale) cached connections for the tool_call allowlist");
                    v
                }
                None => {
                    tracing::warn!(target: "flows", "[flows] connected-toolkit lookup: backend unavailable and no cached snapshot — connected-toolkit allowlist is empty this call");
                    return None;
                }
            }
        }
    };

    Some(
        integrations
            .into_iter()
            .filter(|i| i.connected)
            .map(|i| i.toolkit.to_ascii_lowercase())
            .collect(),
    )
}

/// Effect-aware classification of a Composio `tool_call` slug into the
/// [`CommandClass`] the autonomy-tier gate ([`enforce_node_tier_gate`])
/// evaluates it under.
///
/// Reuses [`curated_scope_for`](crate::openhuman::integrations::composio::providers::curated_scope_for),
/// the same catalog walk `composio::ops`'s `gated_tools` hints use — a
/// registered native provider's `curated_tools()` first, then the static
/// `catalog_for_toolkit` fallback. **Fail-safe by construction:** only a
/// slug that resolves to a curated entry with `ToolScope::Read` maps to
/// `CommandClass::Read` (the one class every tier `Allow`s outright, so a
/// read never parks as a pending approval). Every other outcome — a
/// curated `Write`/`Admin` entry, a toolkit with no catalog entry for this
/// slug, a toolkit with no catalog at all, or an unparseable/empty slug —
/// maps to `CommandClass::Network`, the same class `http_request` uses
/// (prompts under Supervised/Full, blocks under ReadOnly).
///
/// Deliberately does **not** fall back to
/// [`classify_unknown`](crate::openhuman::integrations::composio::providers::classify_unknown)
/// for uncurated slugs: that heuristic is tuned for the *curation*
/// allowlist (`flow_tool_allowed`'s Path B — "is this slug even visible to
/// the agent"), not for deciding whether a real side-effecting call skips
/// a human approval prompt. A "SEARCH"/"GET"-shaped uncurated slug must
/// still prompt until OpenHuman has actually hand-curated it as `Read`.
/// `pub(crate)` so `flows::ops::compute_approval_manifest` can reuse the
/// exact runtime classifier at save time — the manifest must never drift
/// from what actually gates (a parallel re-implementation would list
/// permissions that never prompt, or miss ones that do).
pub(crate) async fn classify_composio_action_for_tier(slug: &str) -> CommandClass {
    use crate::openhuman::integrations::composio::providers::{curated_scope_for, ToolScope};

    match curated_scope_for(slug) {
        Some(ToolScope::Read) => CommandClass::Read,
        Some(ToolScope::Write) | Some(ToolScope::Admin) | None => CommandClass::Network,
    }
}

/// Deny-by-default curation gate for a flow `tool_call` slug (see
/// [`flow_tool_allowed`] for the decision matrix). Fetches the user's live
/// connected-toolkit set only when the slug's toolkit has no static catalog.
pub(crate) async fn is_curated_flow_tool(config: &Config, slug: &str) -> bool {
    let connected = if slug_needs_connected_set(slug) {
        connected_toolkit_slugs(config).await
    } else {
        None
    };
    flow_tool_allowed(config, slug, connected.as_deref()).await
}

/// Finds the connected account a Composio `connection_id` refers to within a
/// live connected-integrations snapshot, returning `(toolkit, display_label)`.
/// UI-safe: the label is the pre-derived [`IntegrationConnection::label`], never
/// a raw account-identity field. Pure over the snapshot so it is unit-testable.
fn resolve_account<'a>(
    integrations: &'a [crate::openhuman::integrations::composio::ConnectedIntegration],
    connection_id: &str,
) -> Option<(&'a str, Option<&'a str>)> {
    integrations.iter().find_map(|integ| {
        integ
            .connections
            .iter()
            .find(|c| c.connection_id == connection_id)
            .map(|c| (integ.toolkit.as_str(), c.label.as_deref()))
    })
}

/// Resolves a Composio `connection_id` to the specific connected account it
/// targets, for logging "which account was used". Best-effort: `None` when the
/// id isn't found in the user's live connected accounts (stale cache / foreign
/// id) or the backend is unreachable.
pub(crate) async fn resolve_composio_account(
    config: &Config,
    connection_id: &str,
) -> Option<(String, Option<String>)> {
    let integrations =
        crate::openhuman::integrations::composio::fetch_connected_integrations(config).await;
    resolve_account(&integrations, connection_id)
        .map(|(toolkit, label)| (toolkit.to_string(), label.map(str::to_string)))
}

/// [`ToolInvoker`] adapter over Composio (`src/openhuman/integrations/composio/client.rs`).
///
/// **B2 (closes two B1 deviations, see
/// `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §4-5):**
/// - **Curation + scope (hard allowlist)**: every call is checked against
///   [`is_curated_flow_tool`] — a deny-by-default gate that only allows a
///   slug resolving to a *known, curated* toolkit action, unlike the general
///   agent tool-call path's more permissive
///   `memory_sync::composio::providers::is_action_visible_with_pref` (see
///   [`is_curated_flow_tool`]'s doc for why the two differ). A non-curated /
///   unrecognized / out-of-scope slug is rejected with
///   `EngineError::Capability("tool not permitted: <slug>")` before any
///   Composio call. **As of tinyflows 0.3 this is load-bearing, not merely
///   defense-in-depth**: integration-node config (including `slug`) is now
///   `=`-expression evaluated against upstream/trigger data before `invoke`,
///   so a trigger payload *can* influence which tool a `=`-derived slug
///   resolves to. The curation gate runs on the **resolved** slug (verified:
///   a `=item.tool`-derived unknown slug is rejected here before Composio),
///   constraining any data-derived tool to the user's curated, in-scope,
///   connected set — and it still closes the case where an author hand-types
///   an arbitrary/typo'd slug.
/// - **connection_ref**: `conn` (`"composio:<toolkit>:<connection_id>"`) is
///   parsed and forwarded in both modes: Direct passes it to Composio's
///   execute endpoint, while Backend includes it as `connectionId` in the
///   authenticated `/agent-integrations/composio/execute` request. Omitting a
///   ref deliberately keeps the ambient-account behavior. A supplied but
///   stale/foreign id is still forwarded exactly so the provider rejects it;
///   it must never degrade to the ambient account (fixes #5751 / E-m3).
/// - **Trust gate**: invocation is also routed through the OpenHuman
///   `ApprovalGate` (mirrors `tinyagents/middleware.rs::ApprovalSecurityMiddleware`)
///   before dispatch, closing the Codex P1 finding that flow tool nodes
///   bypassed the Network/tool approval gate entirely. `ops::flows_run` /
///   `flows_resume` scope a `TrustedAutomation { Workflow }` origin around
///   the whole run, so the gate either auto-allows (pre-declared trust root)
///   or — when the flow's `require_approval` is set — parks for a real
///   decision. No gate installed (unit tests, some hosts) means no gating,
///   same as the existing agent tool-loop middleware.
///
/// // SECURITY NOTE (tinyflows 0.3, now the pinned version): integration nodes
/// // `=`-resolve config from upstream/trigger data, so a trigger-driven flow
/// // whose `slug`/`url` is `=`-derived lets untrusted trigger data pick *which*
/// // curated + in-scope + connected tool/endpoint runs (blast radius bounded by
/// // the curation + scope + connection checks above and the approval gate).
/// // For such flows authors should set `require_approval`. FOLLOW-UP: auto-force
/// // approval when a trigger-driven run's tool/http config contains `=`-exprs.
pub struct OpenHumanTools {
    pub config: Arc<Config>,
    pub security: Arc<SecurityPolicy>,
}

/// Required-arg preflight for a Composio `tool_call`: fails **before** the
/// Composio dispatch when a required arg is missing or resolved to `null`,
/// with a message that names the field and the likely fix — instead of letting
/// the raw provider error surface from deep inside the call.
///
/// Two independent halves:
///
/// 1. The **static** rules `prepare_execute_arguments` already enforces at
///    dispatch (`GMAIL_SEND_EMAIL` needs a recipient, `GOOGLECALENDAR_*` time
///    bounds must be RFC 3339, …). These need no catalog, no network and no
///    API key, so they always run.
/// 2. The **catalog-driven** required-arg list, which is best-effort: when the
///    action's schema cannot be looked up that half is skipped (never blocks
///    on catalog availability).
///
/// Before #6154 only (2) existed, so a host with no reachable Composio
/// catalog — the common case in a dry run, and any offline/unkeyed run — had
/// a preflight that silently passed everything and left the failure to
/// surface from inside the dispatch instead.
pub(crate) async fn preflight_composio_args(
    config: &Config,
    slug: &str,
    args: &Value,
) -> Result<()> {
    // (1) Static rules — the same validation the Composio dispatch runs, hoisted
    // ahead of it. Only the `Err` matters here; the normalized arguments it
    // returns are recomputed (and used) at dispatch.
    if let Err(e) =
        crate::openhuman::integrations::composio::execute_prepare::prepare_execute_arguments(
            slug,
            Some(args.clone()),
        )
    {
        tracing::warn!(target: "flows", %slug, error = %e, "[flows] preflight: static arg rule rejected the call — failing before dispatch");
        return Err(EngineError::Capability(format!("tool_call `{slug}`: {e}")));
    }

    // (2) Catalog-driven required args.
    let Some(required) = composio_required_args(config, slug).await else {
        tracing::info!(target: "flows", %slug, "[flows] preflight: no live catalog schema for action — required-arg check limited to static rules");
        return Ok(());
    };
    let missing = missing_required_args(&required, args);
    if missing.is_empty() {
        tracing::debug!(target: "flows", %slug, "[flows] preflight: all required args present");
        return Ok(());
    }
    tracing::warn!(target: "flows", %slug, ?missing, "[flows] preflight: required arg(s) missing or null — failing before dispatch");
    let list = missing
        .iter()
        .map(|m| format!("`{m}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let first = &missing[0];
    Err(EngineError::Capability(format!(
        "tool_call `{slug}`: required arg(s) {list} missing or resolved to null — wire each from \
         an upstream node's output, e.g. \"{first}\": \"=nodes.<node_id>.item.json.<field>\" \
         (drop `.json` only if `<node_id>` is a code/transform/split_out/merge/trigger node — \
         `agent`/`tool_call`/`http_request` nodes wrap their output in a `{{json,text,raw}}` \
         envelope). If the value comes from an agent node, give that agent an output schema \
         (config.output_parser.schema) so its fields are addressable."
    )))
}

/// Turns a Composio execute response that reports a provider-side failure
/// into a real capability error.
///
/// The Composio execute endpoint is a "successful HTTP request describing an
/// unsuccessful tool call" API: a transport-level failure (network error, 5xx,
/// bad JSON) already surfaces as `Err` via `?` in [`OpenHumanTools::invoke`],
/// but a 200 response whose body is `{successful: false, error: "..."}` (e.g.
/// Slack rejecting `SLACK_SEND_MESSAGE` with a 400 "Invalid request data")
/// comes back as `Ok(ComposioExecuteResponse)` — nothing downstream ever
/// inspected `successful`, so the tinyflows engine recorded the step (and
/// therefore the run) as `Success`/`"completed"` even though the requested
/// action never actually happened upstream.
///
/// Called on every Composio response (never on native `oh:` tool results,
/// which don't carry this envelope and return earlier in `invoke`). A
/// genuinely successful response (`successful: true`) passes through
/// unchanged; an unsuccessful one becomes `Err(EngineError::Capability(_))`,
/// which the engine turns into `StepStatus::Error` and — via
/// `degrade_completed_status` — a degraded/failed run instead of a false
/// "Completed".
pub(crate) fn reject_unsuccessful_composio_response(
    slug: &str,
    resp: crate::openhuman::integrations::composio::ComposioExecuteResponse,
) -> Result<crate::openhuman::integrations::composio::ComposioExecuteResponse> {
    if resp.successful {
        return Ok(resp);
    }
    let detail = resp
        .error
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .unwrap_or("no error detail returned by the provider");
    Err(EngineError::Capability(format!(
        "tool_call `{slug}` failed at the connected provider: {detail}"
    )))
}

/// Native-tool analogue of [`reject_unsuccessful_composio_response`].
///
/// `execute_tool` returns `Ok(outcome)` for a tool that *ran* but *failed* —
/// the failure rides on [`ToolResult::is_error`] (quota exceeded, file missing,
/// no integration client configured). Nothing downstream inspected that flag,
/// so the tinyflows engine recorded the step — and therefore the run — as
/// `Success` even though the tool never did its job. Concretely: a file-upload
/// step could fail, the next node would bind a `null` URL, and the run still
/// reported "completed".
///
/// Mirrors the Composio branch's contract so both paths turn a failed step into
/// `StepStatus::Error` (and, via `degrade_completed_status`, a failed run)
/// rather than a false "Completed".
pub(crate) fn reject_failed_native_tool_result(
    slug: &str,
    result: &crate::openhuman::skills::types::ToolResult,
) -> Result<()> {
    if !result.is_error {
        return Ok(());
    }
    let rendered = result.output();
    let detail = match rendered.trim() {
        "" => "no error detail returned by the tool",
        d => d,
    };
    tracing::warn!(
        target: "flows",
        %slug,
        %detail,
        "[flows] tool_call: native tool reported is_error — failing the step"
    );
    Err(EngineError::Capability(format!(
        "tool_call `{slug}` failed: {detail}"
    )))
}

/// Unwraps a native (`oh:`) tool's [`ToolResult`] into the value a downstream
/// node actually binds against.
///
/// Serializing the `ToolResult` verbatim (the previous behavior) placed the
/// whole envelope on `item.json`, so reaching a field required
/// `=nodes.<id>.item.json.content[0].data.<field>`. That expression does
/// evaluate, but no builder agent ever emits it, which left native tools
/// effectively unbindable in practice.
///
/// A lone `Json` block therefore returns its `data` directly, so a native node
/// binds with the same `=nodes.<id>.item.json.<field>` shape used everywhere
/// else. Anything else (plain text, or mixed/multiple blocks) collapses to
/// `{ "text": <output()> }` so there is always a predictable field to bind.
pub(crate) fn native_tool_payload(result: &crate::openhuman::skills::types::ToolResult) -> Value {
    use crate::openhuman::skills::types::ToolContent;
    match result.content.as_slice() {
        [ToolContent::Json { data }] => data.clone(),
        _ => json!({ "text": result.output() }),
    }
}

/// A [`ToolInvoker`] decorator that runs the host's Composio required-arg
/// preflight before delegating to `inner`.
///
/// Used by `dry_run_workflow`: the dry-run path executes against tinyflows'
/// echo mocks, which would happily accept a `null` required arg — wrapping
/// the mock invoker with this makes the wiring check actually check wiring,
/// so an unwired required arg fails the dry run with the same actionable
/// message a real run would produce.
pub struct PreflightToolInvoker {
    /// Host config, for the Composio schema lookup.
    pub config: Arc<Config>,
    /// The delegate that performs the actual invocation (e.g. the mock).
    pub inner: Arc<dyn ToolInvoker>,
}

#[async_trait]
impl ToolInvoker for PreflightToolInvoker {
    async fn invoke(&self, slug: &str, args: Value, conn: Option<&str>) -> Result<Value> {
        // Ask the backend that owns this slug to validate the args. Previously
        // this called the Composio preflight directly behind a
        // `!slug.starts_with("oh:")` test, which duplicated the dispatch rule
        // and hard-wired one namespace's knowledge into a generic wrapper.
        if let Some(backend) = tools::backend_for(slug) {
            backend.preflight(&self.config, slug, &args).await?;
        }
        self.inner.invoke(slug, args, conn).await
    }
}

#[async_trait]
impl ToolInvoker for OpenHumanTools {
    async fn invoke(&self, slug: &str, args: Value, conn: Option<&str>) -> Result<Value> {
        let ctx = tools::ToolCallCtx {
            config: &self.config,
            security: &self.security,
        };
        match tools::backend_for(slug) {
            Some(backend) => backend.invoke(&ctx, slug, args, conn).await,
            None => Err(tools::unclaimed_slug_error(slug)),
        }
    }
}

/// Builds the [`Capabilities`] bundle for one run, wiring each supported
/// host-injected traits to a real OpenHuman adapter (see each adapter above,
/// and [`super::memory_adapter::OpenHumanMemory`] for `memory`, for its
/// contract).
///
/// `state_namespace` scopes the [`FlowStateStore`] KV so two saved flows that
/// use the same state key never read or overwrite each other — callers pass a
/// per-flow namespace (e.g. `"flow:<id>"`). Note this is **not** the same
/// namespace `OpenHumanMemory` writes flow-scoped memory under — that one is
/// derived independently from the run's trusted origin via
/// `flows::flow_namespace`, so the two never need to agree on separator
/// conventions.
pub fn build_capabilities(config: Arc<Config>, state_namespace: impl Into<String>) -> Capabilities {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let http_config = config.http_request.clone();
    let http_creds = Arc::new(HttpCredentialsStore::from_config(&config));

    Capabilities {
        llm: Arc::new(OpenHumanLlm {
            config: config.clone(),
        }),
        tools: Arc::new(OpenHumanTools {
            config: config.clone(),
            security: security.clone(),
        }),
        http: Arc::new(OpenHumanHttp {
            security: security.clone(),
            http_config,
            http_creds,
        }),
        code: Arc::new(OpenHumanCode {
            config: config.clone(),
            security: security.clone(),
        }),
        state: Arc::new(FlowStateStore {
            config: config.clone(),
            namespace: state_namespace.into(),
        }),
        agent: Some(Arc::new(OpenHumanAgentRunner {
            config: config.clone(),
        })),
        // Shell execution needs a dedicated OpenHuman adapter that applies the
        // host's autonomy and sandbox policy. Keep the capability unavailable
        // until that boundary exists rather than inheriting ambient process
        // access from the workflow engine.
        shell: None,
        // `spawn`/`gate` overlap only when a TaskRunner is injected. With
        // `None` the engine still produces the right answer — `spawn` runs its
        // work inline and hands back a settled ticket — so the failure mode is
        // a silent loss of concurrency rather than an error, which is exactly
        // the kind that survives a smoke test. Flow runs are already on tokio,
        // so take the crate's tokio-backed runner. It is in-process only:
        // tickets do not survive a restart, which is the right bound for work
        // a single run collects at its own gate.
        tasks: Some(Arc::new(TokioTaskRunner::new())),
        // OpenHuman already persists `RunOutcome::pending_approvals` and
        // resumes named gates through `flows_resume`. Leaving the optional
        // provider unset deliberately selects Tinyflows' compatible fallback
        // instead of creating a second review store beside that surface.
        approvals: None,
        memory: Some(Arc::new(
            crate::openhuman::flows::tinyflows::memory_adapter::OpenHumanMemory {
                config: config.clone(),
                security,
            },
        )),
        resolver: Arc::new(OpenHumanWorkflowResolver { config }),
    }
}

/// Opens the durable, cross-process checkpointer a `flows_run` uses via
/// `tinyflows::engine::run_with_checkpointer` — this host's
/// [`SqliteCheckpointer`], stored under `<workspace_dir>/flows/checkpoints.db`.
///
/// It became host-owned when tinyflows vendored its state-graph runtime and
/// dropped the SQLite backend with it (tinyflows PR #43). The port keeps the
/// schema and SQL byte-identical, so an existing `checkpoints.db` — and any
/// run interrupted before the upgrade — resumes unchanged. See
/// [`super::super::checkpoint_sqlite`].
pub fn open_flow_checkpointer(
    config: &Config,
) -> anyhow::Result<Arc<dyn tinyflows::engine::Checkpointer<serde_json::Value>>> {
    let db_path = config.workspace_dir.join("flows").join("checkpoints.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create flows directory: {}", parent.display()))?;
    }
    tracing::debug!(target: "flows", db = %db_path.display(), "[flows] opening checkpointer");
    Ok(Arc::new(
        SqliteCheckpointer::<serde_json::Value>::open(&db_path)
            .with_context(|| format!("Failed to open flows checkpointer: {}", db_path.display()))?,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
