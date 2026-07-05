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

use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents::graph::SqliteCheckpointer;
use tinyflows::caps::{
    Capabilities, CodeLanguage, CodeRunner, HttpClient, LlmProvider, StateStore, ToolInvoker,
    WorkflowResolver,
};
use tinyflows::error::{EngineError, Result};
use tinyflows::model::WorkflowGraph;

use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, ComposioClientKind,
};
use crate::openhuman::config::{Config, HttpRequestConfig};
use crate::openhuman::credentials::{HttpCredential, HttpCredentialsStore};
use crate::openhuman::flows;
use crate::openhuman::inference::provider::{
    create_chat_provider, ChatMessage, ChatRequest, UsageInfo,
};
use crate::openhuman::sandbox::{execute_in_sandbox, resolve_sandbox_policy};
use crate::openhuman::security::{
    CommandClass, GateDecision, SecurityPolicy, POLICY_BLOCKED_MARKER,
};
use crate::openhuman::tools::traits::Tool as _;
use crate::openhuman::tools::HttpRequestTool;

/// Maps a `UsageInfo` (not `Serialize`) into a JSON value field-by-field, so
/// [`OpenHumanLlm::complete`] can surface it in its response `Value` without
/// requiring an upstream `Serialize` impl change.
fn usage_to_json(usage: &Option<UsageInfo>) -> Value {
    match usage {
        None => Value::Null,
        Some(u) => json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "context_window": u.context_window,
            "cached_input_tokens": u.cached_input_tokens,
            "cache_creation_tokens": u.cache_creation_tokens,
            "reasoning_tokens": u.reasoning_tokens,
            "charged_amount_usd": u.charged_amount_usd,
        }),
    }
}

/// Hard autonomy-tier gate for an *acting* flow node (Phase 2).
///
/// A flow run scopes a `TrustedAutomation { Workflow }` origin, but the acting
/// power of a run is still bounded by the user's `[autonomy]` tier — the same
/// [`SecurityPolicy`] the agent tool-loop honors (`SecurityPolicy::from_config`
/// off the `[autonomy]` block). Before an `http_request` (Network-class) or
/// `code` (Write-class) node dispatches, we consult
/// [`SecurityPolicy::gate_decision`] for that node's [`CommandClass`] and refuse
/// outright when the tier `Block`s it — mirroring how `curl`/`shell` acting
/// tools gate (`policy.gate_decision(CommandClass::Network)`), so a read-only
/// run can never reach the network or run arbitrary code.
///
/// `Allow`/`Prompt` return `Ok(decision)`: this function only enforces the
/// non-negotiable `Block` floor itself. The caller uses the returned
/// [`GateDecision`] to drive [`gate_call_for_tier`] immediately after, which is
/// what actually performs the `Prompt` round-trip (see that function's doc for
/// why this is not automatic — a saved workflow's own `require_approval` flag
/// would otherwise silently override the tier's `Prompt` decision). The error
/// is prefixed with [`POLICY_BLOCKED_MARKER`] so the harness's repeated-failure
/// middleware recognizes it as a permanent, don't-retry refusal.
fn enforce_node_tier_gate(
    security: &SecurityPolicy,
    class: CommandClass,
    node: &str,
) -> Result<GateDecision> {
    let decision = security.gate_decision(class);
    tracing::debug!(
        target: "flows",
        node,
        ?class,
        ?decision,
        tier = ?security.autonomy,
        "[flows] node tier gate: evaluating autonomy-tier decision"
    );
    if decision == GateDecision::Block {
        tracing::warn!(
            target: "flows",
            node,
            ?class,
            tier = ?security.autonomy,
            "[flows] node tier gate: BLOCKED by autonomy tier — refusing before dispatch"
        );
        return Err(EngineError::Capability(format!(
            "{POLICY_BLOCKED_MARKER} flows {node} node is not permitted under the current \
             autonomy tier ({:?}): {class:?}-class actions are blocked. Raise the [autonomy] \
             tier to run this node.",
            security.autonomy
        )));
    }
    Ok(decision)
}

/// Dispatches to the process-global [`ApprovalGate`](crate::openhuman::approval::ApprovalGate),
/// escalating a `Prompt`-tier decision into a forced human-in-the-loop round
/// trip regardless of the running flow's own `require_approval` toggle.
///
/// **Why this is needed (Codex P1 finding):** `ApprovalGate::intercept_audited`
/// branches on the scoped [`AgentTurnOrigin`](crate::openhuman::agent::turn_origin::AgentTurnOrigin) —
/// for a `TrustedAutomation { source: Workflow { require_approval: false }, .. }`
/// origin (the default for every saved flow unless the author opts in) it
/// returns `Allow` unconditionally, the same pre-declared-trust-root shortcut a
/// user-authorized cron job gets. That shortcut is correct when the node's
/// autonomy-tier decision was itself `Allow`, but it silently defeats a
/// Supervised-tier `Prompt` decision: without this escalation, a Supervised
/// user's `http_request`/`code` node would run unattended purely because the
/// flow's `require_approval` defaults to `false` — the tier's "ask me" was
/// never actually enforced.
///
/// When `tier_decision` is [`GateDecision::Prompt`] and the current origin is a
/// `Workflow { require_approval: false }` trust root, this scopes a *for this
/// call only* `Workflow { require_approval: true }` origin around
/// `intercept_audited`, forcing the real parking/HITL flow. `GateDecision::Allow`
/// (and any other origin shape) passes through unchanged — existing behavior.
async fn gate_call_for_tier(
    tier_decision: GateDecision,
    tool_name: &str,
    action_summary: &str,
    args_redacted: Value,
) -> (crate::openhuman::approval::GateOutcome, Option<String>) {
    use crate::openhuman::agent::turn_origin;

    let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() else {
        return (crate::openhuman::approval::GateOutcome::Allow, None);
    };

    match escalated_origin_for_prompt(tier_decision, turn_origin::current()) {
        Some(escalated) => {
            tracing::debug!(
                target: "flows",
                tool_name,
                "[flows] node tier gate: tier decision is Prompt — escalating this dispatch to a \
                 forced approval round-trip regardless of the flow's require_approval toggle"
            );
            turn_origin::with_origin(
                escalated,
                gate.intercept_audited(tool_name, action_summary, args_redacted),
            )
            .await
        }
        None => {
            gate.intercept_audited(tool_name, action_summary, args_redacted)
                .await
        }
    }
}

/// Pure decision core of [`gate_call_for_tier`]: when `tier_decision` is
/// [`GateDecision::Prompt`] and `origin` is a `Workflow { require_approval:
/// false }` trust root, returns a clone of that origin with `require_approval`
/// flipped to `true` (the forced escalation). Otherwise returns `None` — the
/// caller then dispatches through the unmodified origin, matching prior
/// behavior. Split out as a free function over plain values (no gate, no
/// task-local read) so the escalation policy is unit-testable without a live
/// `ApprovalGate`.
fn escalated_origin_for_prompt(
    tier_decision: GateDecision,
    origin: Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin>,
) -> Option<crate::openhuman::agent::turn_origin::AgentTurnOrigin> {
    use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

    if tier_decision != GateDecision::Prompt {
        return None;
    }
    match origin {
        Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source:
                TrustedAutomationSource::Workflow {
                    require_approval: false,
                },
        }) => Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Workflow {
                require_approval: true,
            },
        }),
        _ => None,
    }
}

/// [`LlmProvider`] adapter over OpenHuman's inference stack
/// (`src/openhuman/inference/provider/`).
///
/// The `agent` node is single-completion in tinyflows 0.2 (no tool-calling
/// loop, no sub-ports), so `complete` performs exactly one `provider.chat`
/// call and returns its result — no agent loop is driven here.
pub struct OpenHumanLlm {
    pub config: Arc<Config>,
}

#[async_trait]
impl LlmProvider for OpenHumanLlm {
    async fn complete(&self, request: Value, conn: Option<&str>) -> Result<Value> {
        if let Some(c) = conn {
            // B1 does not resolve `connection_ref` to a specific BYOK account —
            // `create_chat_provider` picks the configured provider for `role`.
            tracing::debug!(target: "flows", conn = %c, "[flows] llm conn (not resolved in B1)");
        }

        let role = request
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("summarization");
        let temperature = request
            .get("temperature")
            .and_then(Value::as_f64)
            .unwrap_or(0.7);
        let max_tokens = request
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());

        let messages: Vec<ChatMessage> = match request.get("messages").and_then(Value::as_array) {
            Some(entries) if !entries.is_empty() => entries
                .iter()
                .filter_map(|entry| {
                    let content = entry.get("content").and_then(Value::as_str)?.to_string();
                    let role = entry.get("role").and_then(Value::as_str).unwrap_or("user");
                    Some(match role {
                        "system" => ChatMessage::system(content),
                        "assistant" => ChatMessage::assistant(content),
                        "tool" => ChatMessage::tool(content),
                        _ => ChatMessage::user(content),
                    })
                })
                .collect(),
            _ => {
                let prompt = request
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                vec![ChatMessage::user(prompt)]
            }
        };

        tracing::debug!(
            target: "flows",
            role,
            message_count = messages.len(),
            "[flows] llm.complete: dispatching agent-node completion"
        );

        let (provider, model) = create_chat_provider(role, &self.config)
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        let response = provider
            .chat(
                ChatRequest {
                    messages: &messages,
                    tools: None,
                    stream: None,
                    max_tokens,
                },
                &model,
                temperature,
            )
            .await
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        Ok(json!({
            "text": response.text,
            "tool_calls": response.tool_calls,
            "usage": usage_to_json(&response.usage),
            "reasoning_content": response.reasoning_content,
        }))
    }
}

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

/// Parses a `"http_cred:<name>"` `connection_ref` for [`OpenHumanHttp`]. No
/// host-side HTTP credential store exists yet — this only extracts the name
/// so the adapter can log a clear, actionable warning instead of silently
/// ignoring the reference. See [`OpenHumanHttp::request`]'s doc.
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
/// Returns whether `slug` may be invoked as a flow `tool_call`, given (only when
/// needed) the user's live connected-toolkit slug set.
///
/// Split out from [`is_curated_flow_tool`] as a pure function so the two decision
/// paths are unit-testable without a live Composio backend: `connected_toolkits`
/// is `None` when the toolkit has a static catalog (the connected set is never
/// consulted then) or when the connected set could not be fetched (fail-closed).
async fn flow_tool_allowed(slug: &str, connected_toolkits: Option<&[String]>) -> bool {
    use crate::openhuman::memory_sync::composio::providers::{
        catalog_for_toolkit, find_curated, get_provider, load_user_scope_or_default,
        toolkit_from_slug,
    };

    let Some(toolkit) = toolkit_from_slug(slug) else {
        tracing::debug!(target: "flows", %slug, "[flows] tool_call curation: reject — slug has no extractable toolkit prefix");
        return false;
    };

    // Path A: a toolkit OpenHuman ships a static curated catalog for keeps its
    // strict curated-action + per-user scope gating (unchanged from B2).
    if let Some(catalog) = get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit))
    {
        let Some(curated) = find_curated(catalog, slug) else {
            tracing::debug!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — slug is not a curated action of this toolkit");
            return false;
        };
        let pref = load_user_scope_or_default(&toolkit).await;
        let allowed = pref.allows(curated.scope);
        tracing::debug!(target: "flows", %slug, %toolkit, allowed, "[flows] tool_call curation: static curated catalog decision");
        return allowed;
    }

    // Path B (0.3): no static catalog — allow iff the user has a live ACTIVE
    // Composio connection for this toolkit. Made-up toolkits are never connected.
    match connected_toolkits {
        Some(toolkits) => {
            let connected = toolkits.iter().any(|t| t.eq_ignore_ascii_case(&toolkit));
            tracing::debug!(target: "flows", %slug, %toolkit, connected, "[flows] tool_call curation: live connected-toolkit allowlist decision");
            connected
        }
        None => {
            tracing::warn!(target: "flows", %slug, %toolkit, "[flows] tool_call curation: reject — no static catalog and the connected-toolkit set was unavailable (fail-closed)");
            false
        }
    }
}

/// Whether `slug`'s toolkit lacks a static curated catalog, i.e. the curation
/// decision must consult the user's live connected-toolkit set. Kept cheap and
/// offline (a static `match`) so the common cataloged-toolkit path never pays
/// for a connected-set fetch.
fn slug_needs_connected_set(slug: &str) -> bool {
    use crate::openhuman::memory_sync::composio::providers::{
        catalog_for_toolkit, get_provider, toolkit_from_slug,
    };
    match toolkit_from_slug(slug) {
        Some(toolkit) => get_provider(&toolkit)
            .and_then(|p| p.curated_tools())
            .or_else(|| catalog_for_toolkit(&toolkit))
            .is_none(),
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
    use crate::openhuman::composio::{
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

/// Deny-by-default curation gate for a flow `tool_call` slug (see
/// [`flow_tool_allowed`] for the decision matrix). Fetches the user's live
/// connected-toolkit set only when the slug's toolkit has no static catalog.
async fn is_curated_flow_tool(config: &Config, slug: &str) -> bool {
    let connected = if slug_needs_connected_set(slug) {
        connected_toolkit_slugs(config).await
    } else {
        None
    };
    flow_tool_allowed(slug, connected.as_deref()).await
}

/// Finds the connected account a Composio `connection_id` refers to within a
/// live connected-integrations snapshot, returning `(toolkit, display_label)`.
/// UI-safe: the label is the pre-derived [`IntegrationConnection::label`], never
/// a raw account-identity field. Pure over the snapshot so it is unit-testable.
fn resolve_account<'a>(
    integrations: &'a [crate::openhuman::composio::ConnectedIntegration],
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
async fn resolve_composio_account(
    config: &Config,
    connection_id: &str,
) -> Option<(String, Option<String>)> {
    let integrations = crate::openhuman::composio::fetch_connected_integrations(config).await;
    resolve_account(&integrations, connection_id)
        .map(|(toolkit, label)| (toolkit.to_string(), label.map(str::to_string)))
}

/// [`ToolInvoker`] adapter over Composio (`src/openhuman/composio/client.rs`).
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
///   now parsed and forwarded to `direct_execute` (Composio Direct mode).
///   Backend mode's `execute_tool` still has no per-call account-scoping
///   path — that's a backend API gap, not something this seam can close
///   alone — so a `connection_ref` under Backend mode logs a warning and
///   falls back to the ambient signed-in account (documented stub; see
///   `composio_connection_id`).
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
}

#[async_trait]
impl ToolInvoker for OpenHumanTools {
    async fn invoke(&self, slug: &str, args: Value, conn: Option<&str>) -> Result<Value> {
        // Curation + scope gate — hard allowlist (see [`is_curated_flow_tool`]'s
        // doc for why this differs from the general agent tool-call path).
        // Runs before anything else — a rejected slug never reaches the
        // composio client at all.
        if !is_curated_flow_tool(&self.config, slug).await {
            tracing::warn!(
                target: "flows",
                %slug,
                "[flows] tool_call: rejected — not a recognized curated toolkit action, or out \
                 of the user's configured scope"
            );
            return Err(EngineError::Capability(format!(
                "tool not permitted: {slug}"
            )));
        }

        // Approval gate (see the struct doc). Mirrors
        // `tinyagents/middleware.rs::ApprovalSecurityMiddleware::wrap_tool`'s
        // shape exactly: compute summary/redacted args only when a gate is
        // installed, deny short-circuits before any composio call, allow
        // records an audit id to close out after the call resolves.
        let mut audit_id: Option<String> = None;
        if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
            let summary = crate::openhuman::approval::summarize_action(slug, &args);
            let redacted = crate::openhuman::approval::redact_args(&args);
            let (outcome, request_id) = gate.intercept_audited(slug, &summary, redacted).await;
            match outcome {
                crate::openhuman::approval::GateOutcome::Deny { reason } => {
                    return Err(EngineError::Capability(reason));
                }
                crate::openhuman::approval::GateOutcome::Allow => audit_id = request_id,
            }
        }

        let kind = create_composio_client(&self.config)
            .map_err(|e| EngineError::Capability(e.to_string()))?;
        let args_opt = if args.is_null() { None } else { Some(args) };
        let connection_id = conn.and_then(composio_connection_id);

        // Resolve the connection_ref to the SPECIFIC connected account it names,
        // so we can log which account executes and validate it against the
        // user's live connected set. Ambient-session fallback is used ONLY when
        // no connection_ref was supplied.
        let resolved_account = match connection_id {
            Some(id) => Some((id, resolve_composio_account(&self.config, id).await)),
            None => None,
        };

        tracing::debug!(
            target: "flows",
            %slug,
            mode = kind.mode(),
            has_connection_ref = connection_id.is_some(),
            "[flows] tool_call: invoking composio tool"
        );

        let response = match kind {
            ComposioClientKind::Backend(client) => {
                if let Some((id, resolved)) = &resolved_account {
                    match resolved {
                        Some((toolkit, label)) => tracing::warn!(
                            target: "flows",
                            %slug,
                            connection_id = %id,
                            %toolkit,
                            account = label.as_deref().unwrap_or("<unlabeled>"),
                            "[flows] tool_call: connection_ref resolves to a specific account, but \
                             backend mode has no per-call account-scoping path yet — using the \
                             ambient session account instead (documented stub, see caps.rs's \
                             OpenHumanTools doc)"
                        ),
                        None => tracing::warn!(
                            target: "flows",
                            %slug,
                            connection_id = %id,
                            "[flows] tool_call: connection_ref set but backend mode has no per-call \
                             account-scoping path yet — using the ambient session account \
                             (documented stub, see caps.rs's OpenHumanTools doc)"
                        ),
                    }
                }
                client
                    .execute_tool(slug, args_opt)
                    .await
                    .map_err(|e| EngineError::Capability(e.to_string()))
            }
            ComposioClientKind::Direct(tool) => {
                match &resolved_account {
                    Some((id, Some((toolkit, label)))) => tracing::info!(
                        target: "flows",
                        %slug,
                        connection_id = %id,
                        %toolkit,
                        account = label.as_deref().unwrap_or("<unlabeled>"),
                        "[flows] tool_call: executing against the resolved connected account"
                    ),
                    Some((id, None)) => tracing::warn!(
                        target: "flows",
                        %slug,
                        connection_id = %id,
                        "[flows] tool_call: connection_ref connection_id not found among the user's \
                         live connected accounts (stale cache or foreign id) — forwarding to \
                         Composio Direct mode as-is"
                    ),
                    None => tracing::debug!(
                        target: "flows",
                        %slug,
                        "[flows] tool_call: no connection_ref — using the ambient signed-in account"
                    ),
                }
                direct_execute(
                    &tool,
                    slug,
                    args_opt,
                    &self.config.composio.entity_id,
                    connection_id,
                )
                .await
                .map_err(|e| EngineError::Capability(e.to_string()))
            }
        };

        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
                let exec = if response.is_ok() {
                    crate::openhuman::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    response.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        serde_json::to_value(response?).map_err(|e| EngineError::Capability(e.to_string()))
    }
}

/// [`HttpClient`] adapter over `HttpRequestTool`
/// (`src/openhuman/tools/impl/network/http_request.rs`). Allowlist + DNS-rebind
/// guard live inside `execute`, so this adapter gets them for free.
///
/// **B2:** also routes through the OpenHuman `ApprovalGate` before dispatch
/// (same rationale/shape as [`OpenHumanTools::invoke`] — closes the Codex P1
/// finding that flow HTTP nodes bypassed the Network approval gate).
///
/// **Phase 2 — `http_cred:<name>` resolution:** a `"http_cred:<name>"`
/// `connection_ref` is now resolved against the credentials domain's
/// [`HttpCredentialsStore`] (encrypted-at-rest bearer/basic/header templates).
/// The resolved auth header is injected **server-side** into the outbound
/// request — after the approval gate has already computed its redacted audit
/// summary — so the secret is never surfaced to the approval UI, the flow
/// engine/graph, the node's output, or the logs (only the header *name* and
/// scheme are logged; the value is redacted). A `connection_ref` that names an
/// **unknown** credential fails the request closed (`EngineError::Capability`)
/// rather than silently sending it unauthenticated.
pub struct OpenHumanHttp {
    pub security: Arc<SecurityPolicy>,
    pub http_config: HttpRequestConfig,
    pub http_creds: Arc<HttpCredentialsStore>,
}

/// Resolves an optional HTTP `connection_ref` to the stored credential to
/// inject. Split out as a free function (over the store, not `&self`) so the
/// resolve/fail-closed policy is unit-testable without constructing a full
/// [`OpenHumanHttp`] adapter.
///
/// - `None` conn, or a `connection_ref` whose prefix isn't `http_cred:` →
///   `Ok(None)` (no credential to inject; a non-`http_cred:` prefix is logged
///   and ignored, matching the pre-Phase-2 behavior).
/// - a `http_cred:<name>` naming a **known** credential → `Ok(Some(cred))`
///   (secret-bearing — the caller injects it server-side, never logs it).
/// - a `http_cred:<name>` naming an **unknown** credential, a malformed
///   (empty/whitespace-only) name, or a store error → `Err` — the request
///   must fail closed, never proceed unauthenticated. Distinguishing "no
///   `http_cred:` prefix at all" from "`http_cred:` prefix with a malformed
///   name" matters: [`http_cred_name`] collapses both to `None`, which would
///   otherwise let a typo'd or data-derived empty ref (e.g. `"http_cred:"`)
///   silently fall through to an unauthenticated request (Codex P2 finding).
fn resolve_http_credential(
    store: &HttpCredentialsStore,
    conn: Option<&str>,
) -> Result<Option<HttpCredential>> {
    let Some(conn) = conn else {
        return Ok(None);
    };
    if conn.strip_prefix("http_cred:").is_none() {
        tracing::debug!(target: "flows", %conn, "[flows] http conn: unrecognized connection_ref prefix (expected `http_cred:<name>`) — ignoring");
        return Ok(None);
    }
    let Some(name) = http_cred_name(conn) else {
        tracing::warn!(
            target: "flows",
            %conn,
            "[flows] http_request: connection_ref has the `http_cred:` prefix but no credential \
             name — failing the request closed rather than sending it unauthenticated"
        );
        return Err(EngineError::Capability(format!(
            "http_request connection_ref has a malformed http_cred name: {conn:?}"
        )));
    };

    match store.get(name) {
        Ok(Some(cred)) => {
            tracing::debug!(
                target: "flows",
                cred = %name,
                scheme = cred.scheme.as_str(),
                "[flows] http_request: resolved http_cred (secret redacted)"
            );
            Ok(Some(cred))
        }
        Ok(None) => {
            tracing::warn!(
                target: "flows",
                cred = %name,
                "[flows] http_request: connection_ref names an unknown http_cred — failing the \
                 request closed rather than sending it unauthenticated"
            );
            Err(EngineError::Capability(format!(
                "http_request connection_ref names an unknown http_cred: {name}"
            )))
        }
        Err(e) => {
            tracing::error!(
                target: "flows",
                cred = %name,
                error = %e,
                "[flows] http_request: failed to resolve http_cred from the store"
            );
            Err(EngineError::Capability(format!(
                "failed to resolve http_cred '{name}': {e}"
            )))
        }
    }
}

/// Merges a resolved credential's auth header into the outbound `request`'s
/// `headers` object (creating it when absent), returning the header **name**
/// that was injected for redacted logging. The header value carries the secret
/// and is placed only into the request handed to `HttpRequestTool` — it is
/// never logged or returned. An explicit stored credential wins over any inline
/// same-named header the flow author set.
fn inject_http_credential(request: &mut Value, cred: &HttpCredential) -> Result<String> {
    let (header_name, header_value) = cred
        .to_header()
        .map_err(|e| EngineError::Capability(e.to_string()))?;

    let obj = request.as_object_mut().ok_or_else(|| {
        EngineError::Capability("http_request config must be a JSON object".to_string())
    })?;
    let headers_entry = obj
        .entry("headers")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    // A flow author may leave `headers` unset (null) — coerce to an object so
    // the credential still injects. A non-object, non-null `headers` is a
    // malformed config we refuse rather than silently drop the credential.
    if headers_entry.is_null() {
        *headers_entry = Value::Object(serde_json::Map::new());
    }
    let headers_obj = headers_entry.as_object_mut().ok_or_else(|| {
        EngineError::Capability("http_request `headers` must be a JSON object".to_string())
    })?;
    headers_obj.insert(header_name.clone(), Value::String(header_value));

    tracing::info!(
        target: "flows",
        cred = %cred.name,
        scheme = cred.scheme.as_str(),
        header = %header_name,
        "[flows] http_request: injected stored credential header (value redacted)"
    );
    Ok(header_name)
}

#[async_trait]
impl HttpClient for OpenHumanHttp {
    async fn request(&self, mut request: Value, conn: Option<&str>) -> Result<Value> {
        const TOOL_NAME: &str = "flows_http_request";

        // Autonomy-tier gate (Phase 2): an http_request node reaches the network,
        // so it is Network-class. A read-only run `Block`s here and never
        // dispatches; Supervised/Full fall through to the ApprovalGate below.
        // `gate_call_for_tier` is what actually performs the `Prompt` round-trip
        // — it escalates a Supervised `Prompt` decision into a forced approval
        // regardless of the flow's own `require_approval` toggle (Codex P1).
        let tier_decision =
            enforce_node_tier_gate(&self.security, CommandClass::Network, "http_request")?;

        // The approval gate summarizes/redacts the request BEFORE any credential
        // is injected, so a stored secret never lands in the approval UI or
        // audit trail. Injection happens strictly after this point.
        let summary = crate::openhuman::approval::summarize_action(TOOL_NAME, &request);
        let redacted = crate::openhuman::approval::redact_args(&request);
        let (outcome, audit_id) =
            gate_call_for_tier(tier_decision, TOOL_NAME, &summary, redacted).await;
        if let crate::openhuman::approval::GateOutcome::Deny { reason } = outcome {
            return Err(EngineError::Capability(reason));
        }

        // Resolve `http_cred:<name>` to a stored credential and inject its auth
        // header server-side. An unknown name fails the request closed (see
        // `resolve_http_credential`) — we never send it unauthenticated.
        if let Some(cred) = resolve_http_credential(&self.http_creds, conn)? {
            inject_http_credential(&mut request, &cred)?;
        }

        let tool = HttpRequestTool::new(
            self.security.clone(),
            self.http_config.allowed_domains.clone(),
            self.http_config.max_response_size,
            self.http_config.timeout_secs,
        );

        tracing::debug!(
            target: "flows",
            method = ?request.get("method"),
            url = ?request.get("url"),
            "[flows] http_request: dispatching outbound request"
        );

        // `request` is already `{ method, url, headers?, body? }` — the node's
        // config is the request descriptor; `HttpRequestTool::execute` reads
        // only those keys and ignores the rest (e.g. `connection_ref`,
        // `on_error`), so passing the whole config through is safe.
        let result = tool.execute(request).await;

        let outcome: Result<Value> = match result {
            Ok(result) if result.is_error => {
                // `HttpRequestTool::execute` always returns `Ok`, using
                // `is_error` to signal a failed request (non-2xx, DNS/allowlist
                // rejection, timeout, …) — surface that as a capability error
                // so the engine's `on_error`/`retry` policy can act on it.
                Err(EngineError::Capability(result.text()))
            }
            Ok(result) => Ok(json!({ "text": result.text() })),
            Err(e) => Err(EngineError::Capability(e.to_string())),
        };

        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
                let exec = if outcome.is_ok() {
                    crate::openhuman::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    outcome.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        outcome
    }
}

/// [`CodeRunner`] adapter running sandboxed user code via
/// `src/openhuman/sandbox/ops.rs` (`resolve_sandbox_policy` +
/// `execute_in_sandbox`), modeled on
/// `src/openhuman/tools/impl/system/node_exec.rs::run_sandboxed`.
///
/// **Mismatch handled here:** the sandbox runs a shell command string, not a
/// `(language, source, input)` triple. `source` is treated as a function body
/// receiving the serialized `input` items array and returning the node's
/// output — this convention is a B1 design choice (not specified by the
/// crate), matching the mock's "function body" tests
/// (`tinyflows::nodes::integration::code` — e.g. `"source": "return 1;"`).
///
/// Requires `node`/`python3` on the `PATH` the sandbox backend runs under;
/// there is no managed toolchain wiring here (unlike `node_exec`'s
/// `NodeBootstrap`).
///
/// **Phase 2 — autonomy-tier gating:** a `code` node runs arbitrary user code
/// in a sandbox, so it is treated as [`CommandClass::Write`] (state-changing but
/// sandbox-bounded — not inherently catastrophic). Before dispatch it consults
/// [`enforce_node_tier_gate`]: a read-only run `Block`s and never executes; a
/// Supervised run then routes through the `ApprovalGate` (Write ⇒ `Prompt`); a
/// Full run executes silently. This closes the prior gap where the code node had
/// no policy check and no approval gate at all.
pub struct OpenHumanCode {
    pub config: Arc<Config>,
    pub security: Arc<SecurityPolicy>,
}

const CODE_RUN_TIMEOUT_SECS: u64 = 60;

#[async_trait]
impl CodeRunner for OpenHumanCode {
    async fn run(&self, language: CodeLanguage, source: &str, input: Value) -> Result<Value> {
        // Autonomy-tier gate (Phase 2): sandboxed arbitrary-code execution is
        // Write-class. A read-only run `Block`s here and never spawns anything;
        // Supervised/Full fall through to the ApprovalGate below.
        let tier_decision = enforce_node_tier_gate(&self.security, CommandClass::Write, "code")?;

        // Approval gate (mirrors OpenHumanTools/OpenHumanHttp): `gate_call_for_tier`
        // is what turns a Supervised-tier `Prompt` decision into a real human
        // round-trip before any code runs — escalating past the flow's own
        // `require_approval` toggle when the tier itself says "ask me" (Codex P1).
        // A Deny short-circuits. The audit summary is computed on a redacted view
        // of the request, never the raw source secrets, matching the other
        // acting adapters.
        let action = json!({ "language": format!("{language:?}"), "source": source });
        let summary = crate::openhuman::approval::summarize_action("flows_code", &action);
        let redacted = crate::openhuman::approval::redact_args(&action);
        let (gate_outcome, audit_id) =
            gate_call_for_tier(tier_decision, "flows_code", &summary, redacted).await;
        if let crate::openhuman::approval::GateOutcome::Deny { reason } = gate_outcome {
            return Err(EngineError::Capability(reason));
        }

        let outcome: Result<Value> = async {
        let policy = resolve_sandbox_policy(
            SandboxMode::Sandboxed,
            &self.config.action_dir,
            &self.config.runtime,
            false,
        );

        // Work dir lives under `action_dir` (the sandbox workspace root). We keep
        // its path *relative* to `action_dir` so the run command works on every
        // backend: for Local, `execute_in_sandbox`'s `working_dir` is the host
        // cwd; for Docker, `action_dir` is bind-mounted at `/workspace` with
        // `-w /workspace`. Host-absolute paths would not exist inside the
        // container, so we pass `action_dir` as the working dir and reference the
        // script/input by their `action_dir`-relative paths.
        let rel_dir = std::path::Path::new(".flows_code").join(uuid::Uuid::new_v4().to_string());
        let work_dir = self.config.action_dir.join(&rel_dir);
        tokio::fs::create_dir_all(&work_dir)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to create code work dir: {e}")))?;

        let (script_name, interpreter, script_body) = match language {
            CodeLanguage::JavaScript => ("script.js", "node", js_harness(source)),
            CodeLanguage::Python => ("script.py", "python3", python_harness(source)),
        };
        let script_path = work_dir.join(script_name);
        let input_path = work_dir.join("input.json");

        let input_json = serde_json::to_string(&input)
            .map_err(|e| EngineError::Capability(format!("failed to serialize code input: {e}")))?;
        tokio::fs::write(&script_path, script_body)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to write code script: {e}")))?;
        tokio::fs::write(&input_path, input_json)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to write code input: {e}")))?;

        // Backend-agnostic, `action_dir`-relative command paths (see above).
        let rel_script = rel_dir.join(script_name);
        let rel_input = rel_dir.join("input.json");
        let command = format!(
            "{} {} {}",
            shell_quote(interpreter),
            shell_quote(&rel_script.to_string_lossy()),
            shell_quote(&rel_input.to_string_lossy()),
        );

        let mut extra_env = std::collections::HashMap::new();
        if let Ok(host_path) = std::env::var("PATH") {
            extra_env.insert("PATH".to_string(), host_path);
        }

        tracing::debug!(
            target: "flows",
            ?language,
            work_dir = %work_dir.display(),
            "[flows] code: running sandboxed script"
        );

        let exec_result = execute_in_sandbox(
            &policy,
            &command,
            &self.config.action_dir,
            extra_env,
            std::time::Duration::from_secs(CODE_RUN_TIMEOUT_SECS),
        )
        .await;

        // Always clean up the work dir — even when `execute_in_sandbox` itself
        // errors (e.g. a spawn failure) — so temp scripts never leak.
        if let Err(e) = tokio::fs::remove_dir_all(&work_dir).await {
            tracing::debug!(target: "flows", error = %e, "[flows] code: failed to clean up work dir (non-fatal)");
        }

        let result = exec_result
            .map_err(|e| EngineError::Capability(format!("sandbox execution failed: {e}")))?;

        if !result.success() {
            return Err(EngineError::Capability(format!(
                "code node exited non-zero (timed_out={}): {}",
                result.timed_out, result.stderr
            )));
        }

        serde_json::from_str(result.stdout.trim())
            .map_err(|e| EngineError::Capability(format!("code output was not valid JSON: {e}")))
        }
        .await;

        // Close out the approval audit with the run's success/failure (mirrors
        // OpenHumanTools/OpenHumanHttp).
        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
                let exec = if outcome.is_ok() {
                    crate::openhuman::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    outcome.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        outcome
    }
}

/// Wraps user `source` as a function body receiving `input`, executed by Node,
/// printing the JSON result (or `null`) to stdout.
fn js_harness(source: &str) -> String {
    format!(
        "const fs = require('fs');\n\
         const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         const __result__ = (function(input) {{\n{source}\n}})(input);\n\
         process.stdout.write(JSON.stringify(__result__ === undefined ? null : __result__));\n"
    )
}

/// Wraps user `source` as a function body receiving `input`, executed by
/// Python, printing the JSON result (or `null`) to stdout.
fn python_harness(source: &str) -> String {
    let indented: String = if source.trim().is_empty() {
        "    pass".to_string()
    } else {
        source
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "import sys, json\n\
         with open(sys.argv[1]) as __f__:\n    input = json.load(__f__)\n\
         def __user_fn__(input):\n{indented}\n    return None\n\
         __result__ = __user_fn__(input)\n\
         print(json.dumps(__result__))\n"
    )
}

/// POSIX single-quote shell escaping, mirroring
/// `tools/impl/system/node_exec.rs::shell_quote`.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// [`StateStore`] adapter over the `flows::` domain's `flow_state` KV table.
pub struct FlowStateStore {
    pub config: Arc<Config>,
    pub namespace: String,
}

#[async_trait]
impl StateStore for FlowStateStore {
    async fn load(&self, key: &str) -> Result<Option<Value>> {
        flows::kv_get(&self.config, &self.namespace, key)
            .map_err(|e| EngineError::Capability(e.to_string()))
    }

    async fn store(&self, key: &str, value: Value) -> Result<()> {
        flows::kv_set(&self.config, &self.namespace, key, &value)
            .map_err(|e| EngineError::Capability(e.to_string()))
    }
}

/// [`WorkflowResolver`] adapter over the `flows::` domain's saved-flow store.
///
/// A `sub_workflow` node that references a child by `workflow_id` (rather than
/// embedding it inline) resolves through this adapter: the id is a saved flow's
/// id, and [`flows::ops::load_flow_graph`] loads that flow's portable
/// [`WorkflowGraph`] from the SQLite store. An unknown id maps to
/// [`EngineError::Capability`], so the referencing node fails with a clear "no
/// such workflow" error rather than silently no-op'ing.
///
/// The engine bounds recursion (its `MAX_SUB_WORKFLOW_DEPTH` depth counter) and
/// rejects direct self-references before a child runs, so this adapter does not
/// itself need cycle detection — it is a pure id → graph lookup.
pub struct OpenHumanWorkflowResolver {
    pub config: Arc<Config>,
}

#[async_trait]
impl WorkflowResolver for OpenHumanWorkflowResolver {
    async fn resolve(&self, workflow_id: &str) -> Result<WorkflowGraph> {
        tracing::debug!(
            target: "flows",
            %workflow_id,
            "[flows] sub_workflow resolver: resolving workflow_id to a saved flow graph"
        );
        match flows::ops::load_flow_graph(&self.config, workflow_id) {
            Ok(Some(graph)) => {
                tracing::debug!(
                    target: "flows",
                    %workflow_id,
                    node_count = graph.nodes.len(),
                    "[flows] sub_workflow resolver: resolved saved flow graph"
                );
                Ok(graph)
            }
            Ok(None) => {
                tracing::warn!(
                    target: "flows",
                    %workflow_id,
                    "[flows] sub_workflow resolver: no saved flow with that workflow_id"
                );
                Err(EngineError::Capability(format!(
                    "sub_workflow: no saved flow found for workflow_id '{workflow_id}'"
                )))
            }
            Err(e) => {
                tracing::error!(
                    target: "flows",
                    %workflow_id,
                    error = %e,
                    "[flows] sub_workflow resolver: failed to load saved flow graph"
                );
                Err(EngineError::Capability(format!(
                    "sub_workflow: failed to load workflow_id '{workflow_id}': {e}"
                )))
            }
        }
    }
}

/// Builds the [`Capabilities`] bundle for one run, wiring each of the six
/// host-injected traits to a real OpenHuman adapter (see each adapter above for
/// its contract).
///
/// `state_namespace` scopes the [`FlowStateStore`] KV so two saved flows that
/// use the same state key never read or overwrite each other — callers pass a
/// per-flow namespace (e.g. `"flow:<id>"`).
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
        }),
        http: Arc::new(OpenHumanHttp {
            security: security.clone(),
            http_config,
            http_creds,
        }),
        code: Arc::new(OpenHumanCode {
            config: config.clone(),
            security,
        }),
        state: Arc::new(FlowStateStore {
            config: config.clone(),
            namespace: state_namespace.into(),
        }),
        resolver: Arc::new(OpenHumanWorkflowResolver { config }),
    }
}

/// Opens the durable, cross-process checkpointer a `flows_run` uses via
/// `tinyflows::engine::run_with_checkpointer` — the crate's own
/// `tinyagents::graph::SqliteCheckpointer`, stored under
/// `<workspace_dir>/flows/checkpoints.db`.
///
/// Deliberately **not** a bespoke checkpointer: the crate ships its own
/// SQLite-backed `Checkpointer<State>` impl (feature `sqlite`, already enabled
/// on the `tinyagents` dependency), so the seam just opens it — mirrors the
/// construction in `src/openhuman/agent_orchestration/delegation.rs`.
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
mod tests {
    use super::*;
    use crate::openhuman::agent::prompts::types::IntegrationConnection;
    use crate::openhuman::composio::ConnectedIntegration;

    fn integration(
        toolkit: &str,
        connected: bool,
        connections: Vec<IntegrationConnection>,
    ) -> ConnectedIntegration {
        ConnectedIntegration {
            toolkit: toolkit.to_string(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected,
            connections,
            non_active_status: None,
        }
    }

    fn connection(id: &str, label: Option<&str>, is_default: bool) -> IntegrationConnection {
        IntegrationConnection {
            connection_id: id.to_string(),
            label: label.map(str::to_string),
            is_default,
        }
    }

    /// A `composio:<toolkit>:<connection_id>` ref parses to its id and that id
    /// resolves to the SPECIFIC connected account (toolkit + display label) —
    /// not the toolkit's default connection.
    #[test]
    fn connection_ref_resolves_to_the_chosen_account() {
        let integrations = vec![integration(
            "gmail",
            true,
            vec![
                connection("conn_work", Some("work@example.com"), true),
                connection("conn_home", Some("home@example.com"), false),
            ],
        )];

        let id = composio_connection_id("composio:gmail:conn_home")
            .expect("well-formed composio connection_ref should parse");
        assert_eq!(id, "conn_home");

        let (toolkit, label) =
            resolve_account(&integrations, id).expect("id should resolve to a connected account");
        assert_eq!(toolkit, "gmail");
        // The non-default account was chosen — resolution is by id, not default.
        assert_eq!(label, Some("home@example.com"));

        // An id the user does not hold resolves to nothing (best-effort log path).
        assert!(resolve_account(&integrations, "conn_unknown").is_none());
    }

    /// A made-up toolkit that OpenHuman ships no static catalog for and the user
    /// has NOT connected still rejects — even when the connected set is present
    /// but simply doesn't contain it.
    #[tokio::test]
    async fn unknown_toolkit_still_rejects() {
        use crate::openhuman::memory_sync::composio::providers::{
            catalog_for_toolkit, get_provider,
        };
        // Precondition: `flowstestkit` is genuinely uncatalogued, so the decision
        // flows through the connected-set path (not the static curated path).
        assert!(catalog_for_toolkit("flowstestkit").is_none());
        assert!(get_provider("flowstestkit").is_none());

        // No connected set at all → fail-closed reject.
        assert!(!flow_tool_allowed("FLOWSTESTKIT_DO_THING", None).await);
        // Connected set present but does not include this toolkit → reject.
        assert!(!flow_tool_allowed("FLOWSTESTKIT_DO_THING", Some(&["gmail".to_string()])).await);
        // A blank slug is always rejected.
        assert!(!flow_tool_allowed("", Some(&["flowstestkit".to_string()])).await);
    }

    /// A real Composio toolkit OpenHuman ships no static catalog for now PASSES
    /// once the user has an ACTIVE connection for it (the TODO(0.3) fix) — the
    /// exact same slug that rejects above.
    #[tokio::test]
    async fn connected_uncatalogued_toolkit_now_passes() {
        use crate::openhuman::memory_sync::composio::providers::{
            catalog_for_toolkit, get_provider,
        };
        assert!(catalog_for_toolkit("flowstestkit").is_none());
        assert!(get_provider("flowstestkit").is_none());

        assert!(
            flow_tool_allowed("FLOWSTESTKIT_DO_THING", Some(&["flowstestkit".to_string()])).await
        );
        // Case-insensitive match on the toolkit slug.
        assert!(
            flow_tool_allowed("FLOWSTESTKIT_DO_THING", Some(&["FlowsTestKit".to_string()])).await
        );
    }

    fn http_cred_store() -> (tempfile::TempDir, HttpCredentialsStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        // encrypt=true exercises the ChaCha20-Poly1305 at-rest path.
        let store = HttpCredentialsStore::new(dir.path(), true);
        (dir, store)
    }

    /// A `http_cred:<name>` ref resolves to the stored bearer credential and
    /// injects `Authorization: Bearer <token>` onto the outbound request.
    #[test]
    fn http_cred_resolves_and_injects_bearer_header() {
        let (_dir, store) = http_cred_store();
        store
            .upsert(&HttpCredential::bearer("stripe", "sk_live_secret"))
            .unwrap();

        let cred = resolve_http_credential(&store, Some("http_cred:stripe"))
            .expect("resolve ok")
            .expect("credential present");

        let mut request = json!({ "method": "GET", "url": "https://api.example.com" });
        let header = inject_http_credential(&mut request, &cred).unwrap();
        assert_eq!(header, "Authorization");
        assert_eq!(
            request["headers"]["Authorization"],
            json!("Bearer sk_live_secret")
        );
    }

    /// A custom-header credential injects under its own header name while
    /// preserving any headers the flow author already set.
    #[test]
    fn http_cred_injection_preserves_existing_headers() {
        let (_dir, store) = http_cred_store();
        store
            .upsert(&HttpCredential::header("apikey", "X-API-Key", "topsecret"))
            .unwrap();
        let cred = resolve_http_credential(&store, Some("http_cred:apikey"))
            .unwrap()
            .unwrap();

        let mut request = json!({
            "method": "POST",
            "url": "https://api.example.com",
            "headers": { "Content-Type": "application/json" }
        });
        inject_http_credential(&mut request, &cred).unwrap();
        assert_eq!(
            request["headers"]["Content-Type"],
            json!("application/json")
        );
        assert_eq!(request["headers"]["X-API-Key"], json!("topsecret"));
    }

    /// A basic credential injects `Authorization: Basic ...` even when the flow
    /// author set no `headers` object at all.
    #[test]
    fn http_cred_injects_basic_into_absent_headers() {
        let (_dir, store) = http_cred_store();
        store
            .upsert(&HttpCredential::basic("acme", "alice", "pw"))
            .unwrap();
        let cred = resolve_http_credential(&store, Some("http_cred:acme"))
            .unwrap()
            .unwrap();

        let mut request = json!({ "method": "GET", "url": "https://x.example.com" });
        inject_http_credential(&mut request, &cred).unwrap();
        let value = request["headers"]["Authorization"]
            .as_str()
            .expect("Authorization header injected");
        assert!(
            value.starts_with("Basic "),
            "unexpected basic header: {value}"
        );
    }

    /// A `http_cred:<name>` naming a credential that does not exist FAILS the
    /// request closed — it must never proceed silently unauthenticated.
    #[test]
    fn unknown_http_cred_fails_closed() {
        let (_dir, store) = http_cred_store();
        let result = resolve_http_credential(&store, Some("http_cred:ghost"));
        assert!(result.is_err(), "unknown http_cred must fail closed");
    }

    /// A malformed `http_cred:` ref (empty or whitespace-only name) must fail
    /// closed the same as an unknown credential name — it must never be
    /// treated as "no connection_ref" and silently sent unauthenticated
    /// (Codex P2 finding).
    #[test]
    fn malformed_http_cred_name_fails_closed() {
        let (_dir, store) = http_cred_store();
        assert!(
            resolve_http_credential(&store, Some("http_cred:")).is_err(),
            "an empty http_cred name must fail closed, not fall through as no-op"
        );
        assert!(
            resolve_http_credential(&store, Some("http_cred:   ")).is_err(),
            "a whitespace-only http_cred name must fail closed, not fall through as no-op"
        );
    }

    /// No `connection_ref`, or a non-`http_cred:` prefix, injects nothing and
    /// is not an error.
    #[test]
    fn no_http_cred_ref_injects_nothing() {
        let (_dir, store) = http_cred_store();
        assert!(resolve_http_credential(&store, None).unwrap().is_none());
        assert!(
            resolve_http_credential(&store, Some("composio:gmail:conn_1"))
                .unwrap()
                .is_none()
        );
    }

    /// The secret is server-side-only: the approval-gate redaction (computed on
    /// the pre-injection request) never contains it, and after injection it
    /// lives ONLY in the outbound `Authorization` header.
    #[test]
    fn injected_secret_never_reaches_the_audit_redaction() {
        let (_dir, store) = http_cred_store();
        let secret = "sk_live_never_log_me";
        store
            .upsert(&HttpCredential::bearer("stripe", secret))
            .unwrap();
        let cred = resolve_http_credential(&store, Some("http_cred:stripe"))
            .unwrap()
            .unwrap();

        let mut request = json!({ "method": "GET", "url": "https://api.example.com" });
        // Pre-injection redaction — what the approval UI / audit trail sees.
        let redacted = crate::openhuman::approval::redact_args(&request);
        assert!(!serde_json::to_string(&redacted).unwrap().contains(secret));

        inject_http_credential(&mut request, &cred).unwrap();
        assert_eq!(
            request["headers"]["Authorization"],
            json!(format!("Bearer {secret}"))
        );
    }

    // ── Phase 2: autonomy-tier gating of acting nodes ──────────────────────

    fn policy(level: crate::openhuman::security::AutonomyLevel) -> SecurityPolicy {
        SecurityPolicy {
            autonomy: level,
            ..SecurityPolicy::default()
        }
    }

    /// The tier gate an `http_request` (Network-class) node calls: BLOCKED under
    /// a read-only tier, and passed through (to the ApprovalGate) under
    /// supervised/full.
    #[test]
    fn http_request_node_tier_gate_blocks_readonly_allows_higher() {
        use crate::openhuman::security::AutonomyLevel;

        let err = enforce_node_tier_gate(
            &policy(AutonomyLevel::ReadOnly),
            CommandClass::Network,
            "http_request",
        )
        .expect_err("read-only must block a Network-class http_request node");
        if let EngineError::Capability(msg) = err {
            assert!(
                msg.contains(POLICY_BLOCKED_MARKER),
                "read-only block must carry the policy-blocked marker: {msg}"
            );
        } else {
            panic!("expected EngineError::Capability for a blocked node");
        }

        // Supervised/full do not hard-block — they fall through to the
        // ApprovalGate (which performs the Prompt round-trip).
        assert!(enforce_node_tier_gate(
            &policy(AutonomyLevel::Supervised),
            CommandClass::Network,
            "http_request"
        )
        .is_ok());
        assert!(enforce_node_tier_gate(
            &policy(AutonomyLevel::Full),
            CommandClass::Network,
            "http_request"
        )
        .is_ok());
    }

    /// The tier gate a `code` (Write-class) node calls: BLOCKED under read-only,
    /// allowed under full, prompt-able (not blocked) under supervised.
    #[test]
    fn code_node_tier_gate_blocks_readonly_allows_full() {
        use crate::openhuman::security::AutonomyLevel;

        assert!(enforce_node_tier_gate(
            &policy(AutonomyLevel::ReadOnly),
            CommandClass::Write,
            "code"
        )
        .is_err());
        assert!(enforce_node_tier_gate(
            &policy(AutonomyLevel::Supervised),
            CommandClass::Write,
            "code"
        )
        .is_ok());
        assert!(
            enforce_node_tier_gate(&policy(AutonomyLevel::Full), CommandClass::Write, "code")
                .is_ok()
        );
    }

    /// End-to-end at the adapter: an `http_request` node under a read-only tier
    /// is refused BEFORE any network egress (the tier gate fires ahead of the
    /// approval gate, credential resolution, and dispatch).
    #[tokio::test]
    async fn http_adapter_blocks_under_readonly_tier() {
        use crate::openhuman::security::AutonomyLevel;

        let (_dir, creds) = http_cred_store();
        let http = OpenHumanHttp {
            security: Arc::new(policy(AutonomyLevel::ReadOnly)),
            http_config: HttpRequestConfig::default(),
            http_creds: Arc::new(creds),
        };

        let request = json!({ "method": "GET", "url": "https://example.com" });
        let err = http
            .request(request, None)
            .await
            .expect_err("read-only http_request node must be blocked");
        if let EngineError::Capability(msg) = err {
            assert!(
                msg.contains(POLICY_BLOCKED_MARKER),
                "expected a policy-blocked refusal, got: {msg}"
            );
        } else {
            panic!("expected EngineError::Capability");
        }
    }

    // ── Codex P1: Prompt-tier decisions must escalate past a workflow's own
    // require_approval=false default, never silently auto-allow ────────────

    use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

    fn workflow_origin(job_id: &str, require_approval: bool) -> AgentTurnOrigin {
        AgentTurnOrigin::TrustedAutomation {
            job_id: job_id.to_string(),
            source: TrustedAutomationSource::Workflow { require_approval },
        }
    }

    /// A `Prompt` tier decision on a default (`require_approval: false`)
    /// workflow trust root escalates to `require_approval: true` — the forced
    /// human-in-the-loop round trip that closes the Codex P1 finding.
    #[test]
    fn prompt_decision_escalates_default_workflow_origin() {
        let escalated = escalated_origin_for_prompt(
            GateDecision::Prompt,
            Some(workflow_origin("flow-1", false)),
        )
        .expect("a Prompt decision on require_approval=false must escalate");
        assert!(matches!(
            escalated,
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::Workflow {
                    require_approval: true
                },
                ..
            }
        ));
    }

    /// A flow that already opted into `require_approval: true` needs no
    /// escalation — it's already forced through the parking flow.
    #[test]
    fn prompt_decision_does_not_re_escalate_already_gated_workflow() {
        assert!(escalated_origin_for_prompt(
            GateDecision::Prompt,
            Some(workflow_origin("flow-1", true))
        )
        .is_none());
    }

    /// An `Allow` tier decision never escalates, regardless of the workflow's
    /// `require_approval` toggle — Full-tier runs keep running unattended.
    #[test]
    fn allow_decision_never_escalates() {
        assert!(escalated_origin_for_prompt(
            GateDecision::Allow,
            Some(workflow_origin("flow-1", false))
        )
        .is_none());
    }

    /// No scoped origin (or a non-Workflow origin) never escalates — there is
    /// nothing to force through the workflow-specific parking flow.
    #[test]
    fn prompt_decision_does_not_escalate_without_a_workflow_origin() {
        assert!(escalated_origin_for_prompt(GateDecision::Prompt, None).is_none());
    }

    // ── Phase 7: sub_workflow-by-id resolver ───────────────────────────────

    fn resolver_test_config(tmp: &tempfile::TempDir) -> Config {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    fn trigger_only_graph() -> WorkflowGraph {
        use tinyflows::model::{Node, NodeKind};
        WorkflowGraph {
            nodes: vec![Node {
                id: "t".to_string(),
                kind: NodeKind::Trigger,
                type_version: 1,
                name: "Trigger".to_string(),
                config: Value::Null,
                ports: Vec::new(),
                position: None,
            }],
            ..Default::default()
        }
    }

    /// The resolver loads a saved flow's graph by its id — the by-`workflow_id`
    /// sub_workflow path resolves against the real flows store.
    #[tokio::test]
    async fn resolver_loads_saved_flow_graph_by_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = Arc::new(resolver_test_config(&tmp));

        let graph_json = serde_json::to_value(trigger_only_graph()).unwrap();
        let flow = flows::ops::flows_create(&config, "child".to_string(), graph_json, false)
            .await
            .expect("create flow");
        let flow_id = flow.value.id.clone();

        let resolver = OpenHumanWorkflowResolver {
            config: config.clone(),
        };
        let graph = resolver
            .resolve(&flow_id)
            .await
            .expect("resolver should load the saved flow graph");
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "t");
    }

    /// An unknown workflow_id surfaces a capability error naming the id, rather
    /// than silently resolving to nothing.
    #[tokio::test]
    async fn resolver_unknown_id_is_a_capability_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = Arc::new(resolver_test_config(&tmp));
        let resolver = OpenHumanWorkflowResolver { config };

        let err = resolver
            .resolve("does-not-exist")
            .await
            .expect_err("unknown workflow_id must error");
        match err {
            EngineError::Capability(msg) => assert!(
                msg.contains("does-not-exist"),
                "error should name the missing id: {msg}"
            ),
            other => panic!("expected a capability error, got: {other:?}"),
        }
    }
}
