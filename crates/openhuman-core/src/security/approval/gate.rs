//! `ApprovalGate` — middleware between the agent and any tool whose
//! [`tinytools::Tool::external_effect`] returns `true`.
//!
//! Flow (issue #1339):
//! 1. Agent harness calls [`ApprovalGate::intercept`] with the tool
//!    name, a redacted JSON of the arguments, and a short summary.
//! 2. Gate registers the call's posture with TinySecurity. Durable whole-tool
//!    trust is resolved by the module under the exact approval scope;
//!    `autonomy.auto_approve` is not replayed, so revocations cannot be
//!    resurrected from legacy config. `ApproveAlwaysForTool` records a grant
//!    through `GrantTool2` after resolving the current call.
//! 3. Otherwise: persist a row in `pending_approvals`, publish a
//!    [`DomainEvent::ApprovalRequested`] event so the UI can pop a
//!    toast, and park the call on a `oneshot::Sender` keyed by
//!    `request_id`.
//! 4. UI calls `approval_decide` (RPC) which routes through
//!    [`ApprovalGate::decide`] → sends the decision on the oneshot.
//! 5. The parked future wakes with the decision and translates it
//!    into [`GateOutcome::Allow`] / `Deny`.
//!
//! Sessions: the gate is keyed by an internal per-launch UUID
//! (`session-<uuid>`) used purely for audit grouping. This value is
//! generated unconditionally by the caller (see
//! `bootstrap_core_runtime`) and is never derived from the JSON-RPC
//! bearer token or any other credential material — it is safe to
//! persist and to log. Rows from prior launches are intentionally
//! preserved on init — the issue #1339 acceptance criterion requires
//! they survive restart so the UI can show / dismiss orphans.
//! Decisions on orphan rows update the DB but cannot resume a parked
//! future across processes — no side effect can fire across launches,
//! so the security invariant is preserved without auto-purging.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
use tinytools::status::POLICY_DENIED_MARKER;

use super::engine::{
    CallOrigin, CheckInput, Checked, Choice, Delivery, EngineError, Posture, PromptSurface,
    SharedEngine, Variant,
};
use super::types::{
    ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, GateOutcome, PendingApproval,
};

/// Disambiguates why [`ApprovalGate::decide`] returned `Ok(None)`. See
/// [`ApprovalGate::classify_decide_miss`] for the lookup that produces this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecideMiss {
    /// The pending row was already decided, lazily expired, or superseded — a
    /// benign race (TAURI-RUST-5EH). Safe to demote out of Sentry.
    AlreadyResolved,
    /// No row was ever persisted for this request_id — a genuine lost
    /// registration that must stay a Sentry signal.
    NeverRegistered,
}

/// A decision refused by the gate before it reached the store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    /// The request was parked by a different agent than the one deciding it.
    #[error("approval request {request_id} belongs to another agent")]
    WrongAgent { request_id: String },
    /// The decision came from a different profile than the parked request.
    #[error("approval request {request_id} belongs to another tenant")]
    WrongTenant { request_id: String },
}

/// The key `thread_to_request` files a parked chat thread under, keyed by
/// the tenant that parked it: the thread id alone for the process's own
/// sessions, `<agent>\x1f<thread>` for an embedded agent (both unchanged),
/// and the [`tenant_key`](crate::core::runtime::tenant_key) of the thread for
/// a SaaS profile, so two profiles' default agents (which have no agent id)
/// never share a route. A profile key starts with `\x1e`, which no agent id
/// contains, and the desktop and SaaS modes never share a process.
pub(crate) fn thread_route_key(tenant: &crate::core::runtime::Tenant, thread_id: &str) -> String {
    match (&tenant.profile, &tenant.agent) {
        (Some(_), _) => crate::core::runtime::tenant_key(tenant, thread_id),
        (None, Some(agent)) => format!("{agent}\u{1f}{thread_id}"),
        (None, None) => thread_id.to_string(),
    }
}

/// The tenant whose approvals the calling task parks and answers. A SaaS
/// task with no scope gets a tenant nothing else shares, so it can neither
/// find nor clear another user's route.
pub(crate) fn current_route_tenant() -> crate::core::runtime::Tenant {
    crate::core::runtime::tenant::current_tenant_or_isolated("approval_gate")
}

/// Phrase every TTL-expiry denial carries, so a caller that renders its own
/// refusal (the approval resolver, `middleware::approval`) can tell "nobody answered" from
/// "the user said no" without parsing the rest of the reason. See
/// [`is_unanswered_approval_reason`].
pub const APPROVAL_UNANSWERED_PHRASE: &str = "nobody answered the approval prompt";

/// Whether the parked `request_id` was routed to its chat thread from a task
/// detached from that turn (an async-delegated sub-agent, #5499), so its card
/// can outlive the turn. Read by the web-channel surface when it bridges
/// `ApprovalRequested` (the route is recorded before the event is published);
/// `false` when no gate is installed or the request is unknown.
pub fn is_detached_request(request_id: &str) -> bool {
    ApprovalGate::try_global().is_some_and(|gate| gate.request_is_detached(request_id))
}

/// Whether a [`GateOutcome::Deny`] reason is a TTL expiry (the prompt went
/// unanswered) rather than a refusal.
pub fn is_unanswered_approval_reason(reason: &str) -> bool {
    reason.contains(APPROVAL_UNANSWERED_PHRASE)
}

/// Per-turn chat context for routing a parked approval's yes/no reply back to
/// the originating thread. The web channel hands it to the turn it starts
/// (inside an [`ApprovalTurnContext`]); the turn's run context carries it to
/// the gate and to the tools that read it. Absent for non-chat callers (CLI,
/// detached sub-agents) — their approvals are simply not chat-routable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalChatContext {
    pub thread_id: String,
    pub client_id: String,
    /// The turn currently running on this thread, when the caller has one in
    /// scope. Carried through to `external_transfer_pending` (and any other
    /// event this context backs) so the frontend can correlate a disclosure
    /// to the turn that triggered it instead of only the thread.
    pub request_id: Option<String>,
}

/// Per-run flow context (flow-approval-surface, PR2 of the tinyflows
/// approval-surfacing design). `flows::ops::flows_run` / `flows_resume`
/// hand it to the engine's capabilities, alongside the existing
/// `Workflow` [`AgentTurnOrigin`](crate::security::turn_origin::AgentTurnOrigin),
/// so a tool call parked from that run can correlate
/// [`PendingApproval::source_context`](super::types::PendingApproval) back to
/// the exact flow + run (the origin alone only carries `flow_id`, not
/// `run_id`). Absent for every non-flow caller — chat, cron, background
/// jobs, CLI never scope this.
#[derive(Clone, Debug)]
pub struct FlowRunContext {
    pub flow_id: String,
    pub run_id: String,
}

/// What the approval gate needs to know about the turn that owns a gated call,
/// beyond the turn origin: where to route the yes/no reply, whether the turn is
/// the Flow Canvas copilot, and which flow run it belongs to.
///
/// This is an explicit value, handed from the entry point that starts the turn
/// to the gate check (and to the tools that route their own prompts) through
/// the turn's run context. It used to be three ambient task-locals, which meant
/// "no chat context" was indistinguishable from "this task lost its scope".
///
/// Deliberately **not** carried across a detached spawn: a detached sub-agent
/// (`OpenHumanRunContext::detached_child`) starts with the default value, so
/// its parks route via the origin as `detached` and survive the parent turn
/// (#5499). Inline children clone their parent's value.
#[derive(Clone, Debug, Default)]
pub struct ApprovalTurnContext {
    /// Thread/client to surface a parked approval to. `None` for non-chat
    /// callers (CLI, cron, background, detached sub-agents).
    pub chat: Option<ApprovalChatContext>,
    /// The turn is the Flow Canvas copilot's streaming `flows_build` path: the
    /// module gives the park its shorter copilot window. Plain main-chat
    /// `WebChat` turns leave it `false` and keep the module's full window.
    pub copilot_stream: bool,
    /// The flow run the turn belongs to, so a park stamps its
    /// `PendingApproval::source_context` with the exact flow and run. `None`
    /// for every non-flow caller.
    pub flow_run: Option<FlowRunContext>,
}

impl ApprovalTurnContext {
    /// A chat-routed turn.
    pub fn chat(chat: ApprovalChatContext) -> Self {
        Self {
            chat: Some(chat),
            ..Self::default()
        }
    }

    /// Marks the turn as the Flow Canvas copilot's streaming build.
    pub fn with_copilot_stream(mut self) -> Self {
        self.copilot_stream = true;
        self
    }

    /// Binds the turn to one flow run.
    pub fn with_flow_run(mut self, flow_run: FlowRunContext) -> Self {
        self.flow_run = Some(flow_run);
        self
    }

    /// Whether the turn is interactive: a chat surface is attached to it.
    pub fn is_interactive(&self) -> bool {
        self.chat.is_some()
    }
}

/// Parse a chat reply to a parked approval into a binary decision (v1). Only an
/// explicit yes/no answer maps to a decision; anything else returns `None` — the
/// web channel treats `None` as "not an answer", cancels the parked turn, and
/// dispatches the message as a fresh user turn (so the user can redirect).
pub fn parse_approval_reply(message: &str) -> Option<ApprovalDecision> {
    match message.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "ok" | "okay" | "approve" | "approved" | "allow" => {
            Some(ApprovalDecision::ApproveOnce)
        }
        "no" | "n" | "deny" | "denied" => Some(ApprovalDecision::Deny),
        _ => None,
    }
}

static GLOBAL_GATE: OnceLock<Arc<ApprovalGate>> = OnceLock::new();

/// Snapshot of the host-aware boot decision the runtime made when it
/// evaluated `OPENHUMAN_APPROVAL_GATE`. Surfaced to the UI banner via
/// `approval_get_gate_state` so the user sees a banner the *first* time
/// they open the app after an override was honored, not only when a
/// connected socket happens to receive the boot-time domain event.
///
/// Set exactly once on boot from `bootstrap_core_runtime`; subsequent
/// reads return the same snapshot for the lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalGateBootState {
    /// True when the gate was installed at boot.
    pub installed: bool,
    /// True when an `OPENHUMAN_APPROVAL_GATE=0` env override was honored
    /// (CLI / Docker host) — the gate is OFF and external_effect tools
    /// run unprompted. UI banners on this state.
    pub disabled_by_env: bool,
    /// True when an `OPENHUMAN_APPROVAL_GATE=0` env override was observed
    /// but suppressed because the host is the Tauri desktop shell. UI
    /// surfaces a softer one-shot info banner so the user knows the
    /// override was rejected.
    pub override_ignored: bool,
    /// Host tag the boot decision keyed off — `tauri-shell` / `cli` /
    /// `docker`. Pinned strings; downstream consumers may switch on this.
    pub host: &'static str,
}

static BOOT_STATE: OnceLock<ApprovalGateBootState> = OnceLock::new();

/// Record the host-aware boot decision so the UI / RPC layer can read it
/// back. Idempotent — only the first call wins, mirroring the gate
/// `OnceLock` install pattern.
pub fn record_boot_state(state: ApprovalGateBootState) {
    let _ = BOOT_STATE.set(state);
}

/// Read the recorded boot state. Returns `None` when `record_boot_state`
/// was never called (e.g. older test paths that bring up the gate
/// directly without going through `bootstrap_core_runtime`); RPC and UI
/// callers treat that as "no banner needed".
pub fn try_boot_state() -> Option<ApprovalGateBootState> {
    BOOT_STATE.get().copied()
}

/// Routing correlation captured at park time for one `request_id`: the chat
/// thread/client to surface a decision to, plus the gated tool call's
/// provider-assigned call id. Looked up by [`ApprovalGate::take_request_route`]
/// when a decision resolves so `DomainEvent::ApprovalDecided` can carry the
/// same routing the original `ApprovalRequested` did, without re-deriving it
/// from ambient task-locals that may no longer be in scope (a decision can
/// resolve from an RPC call with no chat context of its own).
#[derive(Clone, Debug)]
pub(crate) struct RequestRoute {
    pub(crate) thread_id: Option<String>,
    pub(crate) client_id: Option<String>,
    pub(crate) tool_call_id: Option<String>,
    pub(crate) forced: bool,
    pub(crate) agent_id: Option<String>,
    pub(crate) variant: Variant,
    /// Instance barrier captured at registration, independent of decision context.
    pub(crate) approval_scope: Option<Arc<super::ApprovalScope>>,
    /// The [`thread_route_key`] the request was parked under, so a decision
    /// made outside the parking task's scope clears the right route.
    pub(crate) thread_key: Option<String>,
    /// The park can outlive the chat turn it is shown on — an async-delegated
    /// sub-agent routed through the origin fallback (#5499). Carried so the
    /// replay path re-emits the same `detached` flag the live event had.
    pub(crate) detached: bool,
    /// The tenant that parked the request, which scopes who may list it.
    pub(crate) tenant: crate::core::runtime::Tenant,
    /// What the user is shown for the request: the engine's own redacted
    /// summary and arguments, with the host's routing correlation.
    pub(crate) pending: PendingApproval,
}

/// The requests parked in this process, shared between the gate and the prompt
/// surface the engine calls while it makes a request durable.
#[derive(Default)]
pub(crate) struct Parked {
    waiters: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
    /// [`thread_route_key`] → request_id for the approval currently parked on
    /// that chat thread, so the web channel can route a yes/no reply to
    /// `approval_decide`.
    /// In-memory only (session-scoped — a parked approval doesn't survive a
    /// restart, and the oneshot waiter is in-memory anyway).
    thread_to_request: Mutex<HashMap<String, String>>,
    /// request_id → [`RequestRoute`] for every currently-parked call. Populated
    /// when the engine delivers the prompt, consulted when a decision resolves
    /// (`decide`, and the TTL/channel-drop paths in `gate_intercept.rs`) so
    /// `ApprovalDecided` can mirror the same thread/client/tool_call_id the
    /// original `ApprovalRequested` carried.
    request_routes: Mutex<HashMap<String, RequestRoute>>,
}

/// Coordinator for pending approvals.
///
/// The lifecycle of each approval is the engine's ([`ApprovalEngine`]); the
/// gate owns the verified origin, the user's configured posture, the parked
/// tool-call future, chat routing and the product events.
pub struct ApprovalGate {
    config: Config,
    session_id: String,
    /// A park window shorter than the module's own, when configured
    /// (`security.approval.ttl_secs`). The module owns the default window.
    ttl: Option<Duration>,
    engine: SharedEngine,
    parked: Arc<Parked>,
    /// Once a required terminal audit write fails, external effects fail closed
    /// for the rest of this process. Already completed tool results are untouched.
    audit_failure: AtomicBool,
}

/// What the prompt surface knows about the call it shows requests for.
struct ParkContext {
    chat_thread_id: Option<String>,
    chat_client_id: Option<String>,
    tool_call_id: Option<String>,
    forced: bool,
    variant: Variant,
    agent_id: Option<String>,
    approval_scope: Option<Arc<super::ApprovalScope>>,
    thread_key: Option<String>,
    detached: bool,
    tenant: crate::core::runtime::Tenant,
    source_context: Option<ApprovalSourceContext>,
}

/// The gate's side of one gated call: where the engine shows the prompt, and
/// the waiter the decision resumes the call through.
struct Surface {
    parked: Arc<Parked>,
    context: ParkContext,
    waiter: Mutex<Option<oneshot::Sender<ApprovalDecision>>>,
    /// The request the engine delivered for this call, once it did.
    delivered: Mutex<Option<String>>,
}

/// Tears the parked waiter down even when the surrounding turn future is
/// dropped mid-park.
///
/// `intercept_audited_inner` only runs its cleanup (`evict_waiter` /
/// cancelling the request / routing-map removal) in the arms of its park
/// resolution — i.e. only when the park resolves *normally*. Once a turn
/// future can be torn down *externally* — the harness `max_wall_clock_ms`
/// backstop (#4746) or the outer web backstop (#4751) firing while a tool call
/// is parked — dropping the future skips those arms entirely, leaving the
/// in-memory waiter, the thread routing mappings, and the pending request
/// dangling until it expires. A later yes/no arriving before that expiry would
/// then route to a dead request and return without starting a fresh turn
/// (#4774).
///
/// The guard is created before the engine is asked (the prompt can be
/// delivered while the check is still in flight) and
/// [`disarm`](Self::disarm)ed on every normal exit (the match arm already ran
/// the exact teardown for its outcome), so its `Drop` fires *only* on external
/// cancellation.
struct CancelGuard {
    engine: SharedEngine,
    parked: Arc<Parked>,
    surface: Arc<Surface>,
    armed: bool,
}

impl CancelGuard {
    /// Mark the park as resolved normally so `Drop` becomes a no-op.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(request_id) = self.surface.delivered_request() else {
            return;
        };
        // External teardown: the normal cleanup was skipped. Evict the waiter,
        // drop the routing mapping so a later chat reply is not mis-routed to
        // this now-dead request, and cancel the still-open request.
        self.parked.evict_waiter(&request_id);
        // Only clear the routing entry when it still points at *this* request.
        // On external teardown a replacement turn can park a new approval on the
        // same thread and overwrite the mapping before this guard drops;
        // an unconditional `remove` would delete the *new* request's routing, so
        // the next typed yes/no would fall through as a fresh chat turn instead
        // of resolving the live gate (#4774).
        if let Some(thread_key) = &self.surface.context.thread_key {
            self.parked
                .clear_thread_route_if_owned(thread_key, &request_id);
        }
        if let Some(route) = self.parked.take_request_route(&request_id) {
            BUS.publish(DomainEvent::ApprovalDecided {
                request_id: request_id.clone(),
                tool_name: route.pending.tool_name.clone(),
                decision: ApprovalDecision::Deny.as_str().to_string(),
                thread_id: route.thread_id.clone(),
                client_id: route.client_id.clone(),
                tool_call_id: route.tool_call_id.clone(),
                resolution: Some("cancelled".to_string()),
                agent_id: route.agent_id.clone(),
            });
        }
        let engine = Arc::clone(&self.engine);
        let cancelled = request_id.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(error) = engine.cancel(&cancelled).await {
                        tracing::debug!(
                            request_id = %cancelled,
                            error = %error,
                            "[approval::gate] the engine did not cancel the dropped request"
                        );
                    }
                });
            }
            Err(_) => tracing::debug!(
                request_id = %request_id,
                "[approval::gate] no runtime to cancel the dropped request; it expires on its own"
            ),
        }
        tracing::warn!(
            request_id = %request_id,
            "[approval::gate] parked approval future dropped mid-park (external turn teardown) — \
             evicted waiter, cleared routing, cancelling the request (#4774)"
        );
    }
}

include!("gate_setup.rs");
include!("gate_surface.rs");
include!("gate_intercept.rs");
include!("gate_intercept_decision.rs");
include!("gate_state.rs");
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Surfaces a flow-origin park as a `CoreNotification` (category `Agents`,
/// `kind: "flow-gate-approval"`) with three action buttons matching the
/// [`ApprovalDecision`] variants a flow-scoped approval accepts:
/// `approve_once` / `approve_always_for_flow` / `deny`. Each action's payload
/// carries the same `{kind, request_id, flow_id, tool_name, summary}` shape
/// (plus `run_id`, additive) so the frontend can dispatch straight to
/// `approval_decide` without a second round-trip to fetch the pending row.
///
/// Mirrors `flows::ops::notify_pending_approval` (the tinyflows-native
/// per-node HITL gate's notification) but is a distinct surface: this one
/// fires from the *tool-call* `ApprovalGate`, not the graph's own
/// `require_approval` gate node.
/// `workspace` is the handle and revision of the workspace the flow parked in.
/// This publisher bypasses the notification bridge that stamps workspace
/// identity, and the approval gate itself is process-wide — so without the
/// binding, the banner stays actionable after a workspace switch and approves
/// another workspace's pending call from inside the one the user moved to
/// (#5966). `None` only when the workspace could not be resolved; the caller
/// says why that fails open.
fn publish_flow_gate_notification(
    request_id: &str,
    flow_id: &str,
    run_id: &str,
    tool_name: &str,
    summary: &str,
    workspace: Option<(String, u64)>,
) {
    use crate::desktop::notifications::bus::publish_core_notification;
    use crate::desktop::notifications::types::{
        CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
    };

    const KIND: &str = "flow-gate-approval";
    let base_payload = |action: ApprovalDecision| {
        serde_json::json!({
            "kind": KIND,
            "request_id": request_id,
            "flow_id": flow_id,
            "run_id": run_id,
            "tool_name": tool_name,
            "summary": summary,
            "decision": action.as_str(),
        })
    };

    publish_core_notification(CoreNotificationEvent {
        id: format!("{KIND}:{request_id}"),
        category: CoreNotificationCategory::Agents,
        title: "Workflow needs approval".to_string(),
        body: format!("\"{tool_name}\" — {summary}"),
        deep_link: None,
        timestamp_ms: now_ms(),
        actions: Some(vec![
            CoreNotificationAction {
                action_id: "approve_once".to_string(),
                label: "Approve once".to_string(),
                payload: Some(base_payload(ApprovalDecision::ApproveOnce)),
            },
            CoreNotificationAction {
                action_id: "approve_always_for_flow".to_string(),
                label: "Always allow for this workflow".to_string(),
                payload: Some(base_payload(ApprovalDecision::ApproveAlwaysForFlow)),
            },
            CoreNotificationAction {
                action_id: "deny".to_string(),
                label: "Deny".to_string(),
                payload: Some(base_payload(ApprovalDecision::Deny)),
            },
        ]),
        workspace: workspace.as_ref().map(|(handle, _)| handle.clone()),
        workspace_revision: workspace.map(|(_, revision)| revision),
    });
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
