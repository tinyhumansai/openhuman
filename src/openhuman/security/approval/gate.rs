//! `ApprovalGate` — middleware between the agent and any tool whose
//! [`crate::openhuman::tools::Tool::external_effect`] returns `true`.
//!
//! Flow (issue #1339):
//! 1. Agent harness calls [`ApprovalGate::intercept`] with the tool
//!    name, a redacted JSON of the arguments, and a short summary.
//! 2. Gate checks the user's "Always allow" allowlist
//!    (`autonomy.auto_approve`, read live via
//!    [`crate::openhuman::security::live_policy`]). Hit → `Allow`
//!    immediately. An `ApproveAlwaysForTool` decision adds the tool to
//!    that list via `approval_decide` (config save + policy reload).
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
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::security::POLICY_DENIED_MARKER;

use super::store;
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

/// How long the gate will park a future before timing out and
/// returning `Deny`. 10 minutes matches the default `expires_at`
/// written into the persisted row.
const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(60 * 10);

/// Shorter park window for approvals raised by the Flow Canvas copilot's
/// live-run path — `flows_build` streaming into the copilot pane calling
/// `run_flow` / `resume_flow_run` (PR #5090). A stale ten-minute park on a
/// copilot pane the user may have already navigated away from is a long time
/// to leave a live Slack/Gmail/HTTP node waiting; if nobody approves within
/// three minutes, deny and let the authoring turn continue (the user can
/// still re-trigger the run from the Runs rail). Scoped by
/// [`APPROVAL_COPILOT_STREAM_CONTEXT`]. Deliberately NOT applied to
/// main-chat `WebChat` parks — only `flows::ops::flows_build`'s streaming
/// branch scopes the task-local below.
const COPILOT_APPROVAL_TTL: Duration = Duration::from_secs(180);

/// Per-turn chat context for routing a parked approval's yes/no reply back to
/// the originating thread. The web channel scopes this task-local around the
/// agent run (`web_chat`); because the `run_turn` handler, the
/// tool loop, and `intercept` all run inline (`.await`) within that spawned
/// task, it propagates down to `intercept` with no signature plumbing. Absent
/// for non-chat callers (CLI, sub-agents) — their approvals are simply not
/// chat-routable.
#[derive(Clone, Debug)]
pub struct ApprovalChatContext {
    pub thread_id: String,
    pub client_id: String,
}

tokio::task_local! {
    pub static APPROVAL_CHAT_CONTEXT: ApprovalChatContext;
}

tokio::task_local! {
    /// Marks a park as originating from the Flow Canvas copilot's streaming
    /// `flows_build` path — scoped by `flows::ops::flows_build` around the
    /// streaming `agent.run_single(&prompt)` call, alongside the existing
    /// `AgentTurnOrigin::WebChat` + [`APPROVAL_CHAT_CONTEXT`] double-scope that
    /// path already uses. Presence alone is the signal (no fields needed): when
    /// set, the park window is clamped to [`COPILOT_APPROVAL_TTL`] instead of
    /// the gate's own (possibly env-overridden) TTL. Absent for every other
    /// caller — in particular, plain main-chat `WebChat` turns do not scope
    /// this, so they keep the full [`DEFAULT_APPROVAL_TTL`].
    pub static APPROVAL_COPILOT_STREAM_CONTEXT: ();
}

/// Per-run flow context (flow-approval-surface, PR2 of the tinyflows
/// approval-surfacing design). `flows::ops::flows_run` / `flows_resume`
/// scope this around the engine invocation, alongside the existing
/// `Workflow` [`AgentTurnOrigin`](crate::openhuman::agent::turn_origin::AgentTurnOrigin),
/// so a tool call parked from that run can correlate
/// [`PendingApproval::source_context`](super::types::PendingApproval) back to
/// the exact flow + run (the origin alone only carries `flow_id`, not
/// `run_id`). Absent for every non-flow caller — chat, cron, subconscious,
/// CLI never scope this.
#[derive(Clone, Debug)]
pub struct FlowRunContext {
    pub flow_id: String,
    pub run_id: String,
}

tokio::task_local! {
    pub static APPROVAL_FLOW_RUN_CONTEXT: FlowRunContext;
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

/// Coordinator for pending approvals.
pub struct ApprovalGate {
    config: Config,
    session_id: String,
    ttl: Duration,
    waiters: Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>,
    /// thread_id → request_id for the approval currently parked on that chat
    /// thread, so the web channel can route a yes/no reply to `approval_decide`.
    /// In-memory only (session-scoped — a parked approval doesn't survive a
    /// restart, and the oneshot waiter is in-memory anyway).
    thread_to_request: Mutex<HashMap<String, String>>,
}

/// RAII guard that tears the parked waiter down even when the surrounding turn
/// future is dropped mid-park.
///
/// `intercept_audited_inner` only runs its cleanup (`evict_waiter` /
/// `store::decide(Deny)` / routing-map removal) inside the
/// `tokio::time::timeout(...).await` match arms — i.e. only when the park
/// resolves *normally*. Once a turn future can be torn down *externally* — the
/// harness `max_wall_clock_ms` backstop (#4746) or the outer web backstop
/// (#4751) firing while a tool call is parked — dropping the future skips those
/// arms entirely, leaving the in-memory waiter, the thread routing
/// mappings, and the `pending_approvals` row dangling until the store TTL
/// sweeps them. A later yes/no arriving before that expiry would then route to a
/// dead request and return without starting a fresh turn (#4774).
///
/// The guard is created just before the park await and [`disarm`](Self::disarm)ed
/// on every normal exit (the match arm already ran the exact teardown for its
/// outcome), so its `Drop` fires *only* on external cancellation.
struct WaiterGuard<'a> {
    gate: &'a ApprovalGate,
    request_id: String,
    thread_id: Option<String>,
    armed: bool,
}

impl WaiterGuard<'_> {
    /// Mark the park as resolved normally so `Drop` becomes a no-op.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // External teardown: the normal cleanup was skipped. Evict the waiter,
        // drop the routing mapping so a later chat reply is not
        // mis-routed to this now-dead request, and deny the still-open pending
        // row. `store::decide` is `WHERE decided_at IS NULL`, so a decision that
        // committed in the same instant is honored rather than overwritten.
        self.gate.evict_waiter(&self.request_id);
        // Only clear the routing entry when it still points at *this* request.
        // On external teardown a replacement turn can park a new approval on the
        // same thread and overwrite the mapping before this guard drops;
        // an unconditional `remove` would delete the *new* request's routing, so
        // the next typed yes/no would fall through as a fresh chat turn instead
        // of resolving the live gate (#4774).
        if let Some(thread_id) = &self.thread_id {
            self.gate
                .clear_thread_route_if_owned(thread_id, &self.request_id);
        }
        let _ = store::decide(&self.gate.config, &self.request_id, ApprovalDecision::Deny);
        tracing::warn!(
            request_id = %self.request_id,
            "[approval::gate] parked approval future dropped mid-park (external turn teardown) — \
             evicted waiter, cleared routing, denied pending row (#4774)"
        );
    }
}

include!("gate_setup.rs");
include!("gate_intercept.rs");
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
    use crate::openhuman::desktop::notifications::bus::publish_core_notification;
    use crate::openhuman::desktop::notifications::types::{
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
