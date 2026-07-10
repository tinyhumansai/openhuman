//! Types shared between the `ApprovalGate`, the SQLite store, and the
//! RPC layer. Kept narrow so the gate, the store, and the RPC ops can
//! evolve independently without circular imports through `mod.rs`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A tool call that has been intercepted and is awaiting a user
/// decision. Persisted in `pending_approvals` and surfaced to the UI
/// via `approval_list_pending`.
///
/// Note: this type intentionally does not expose a `session_id`. Session
/// provenance is an internal correlation token owned by `ApprovalGate`
/// and the persistence layer; surfacing it on the public type made it
/// too easy for callers to log or serialize a value that historically
/// derived from credential material (the JSON-RPC bearer token). The
/// underlying column is retained in SQLite for downgrade safety but no
/// longer carries any credential-shaped value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PendingApproval {
    pub request_id: String,
    pub tool_name: String,
    /// Short human-readable summary (scrubbed of PII / chat content
    /// per `feedback_redact_paths_and_ids_in_public.md`).
    pub action_summary: String,
    /// Redacted JSON arguments — counts/shape only, no raw message
    /// bodies, per `feedback_pr_no_chat_content.md`.
    pub args_redacted: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Correlation context for a park raised from a non-chat source (e.g. a
    /// `tinyflows` workflow run). `None` for the plain chat-routed path (and
    /// for every other caller that never scopes a matching context) — kept
    /// optional and additive so the chat path's wire shape never changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_context: Option<ApprovalSourceContext>,
}

impl PendingApproval {
    /// Construct a [`PendingApproval`]. Provided so integration tests and
    /// external callers are not blocked by the `#[non_exhaustive]` attribute.
    pub fn new(
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        action_summary: impl Into<String>,
        args_redacted: serde_json::Value,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            tool_name: tool_name.into(),
            action_summary: action_summary.into(),
            args_redacted,
            created_at: Utc::now(),
            expires_at,
            source_context: None,
        }
    }

    /// Attach an [`ApprovalSourceContext`] — used by
    /// [`super::gate::ApprovalGate`] when parking a `Workflow`-origin tool
    /// call so the row (and the `approval_list_pending` JSON the frontend
    /// reads) carries the flow/run correlation.
    pub fn with_source_context(mut self, ctx: ApprovalSourceContext) -> Self {
        self.source_context = Some(ctx);
        self
    }
}

/// Correlation context for a [`PendingApproval`] raised from a source other
/// than a live chat turn. Serialized internally-tagged on `kind` so the
/// frontend can discriminate on a single field:
/// `{ "kind": "flow", "flow_id": ..., "run_id": ..., "node_id": ... }`.
///
/// Currently only the `Flow` source exists (a `tinyflows` workflow run,
/// issue B2/flow-approval-surface) — widen this enum if another correlated,
/// non-chat approval surface needs the same treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalSourceContext {
    Flow {
        /// The `flows::Flow` id whose run parked this tool call.
        flow_id: String,
        /// The run's stable identifier — the tinyflows checkpointer thread
        /// id (`flows::store::FlowRun.id` / `flows::ops::flows_run`'s
        /// `thread_id`) — so a decision (or a per-flow "approve always"
        /// trust grant) can be tied back to the exact run.
        run_id: String,
        /// The graph node that dispatched the call, when known. Not yet
        /// populated by the gate (the tool-call dispatch path doesn't thread
        /// a node id down to `ApprovalGate::intercept_audited` today) —
        /// reserved for a follow-up that does.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },
}

/// Durable audit row for an approval request after a decision.
///
/// See [`PendingApproval`] for the rationale behind omitting
/// `session_id` from the public shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApprovalAuditEntry {
    pub request_id: String,
    pub tool_name: String,
    pub action_summary: String,
    pub args_redacted: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub decided_at: DateTime<Utc>,
    pub decision: ApprovalDecision,
}

/// User's decision on a pending approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Run the call this once; future calls of the same tool will be
    /// gated again.
    ApproveOnce,
    /// Run the call AND add the tool to the session-scoped allowlist
    /// so subsequent calls of the same tool skip the gate until the
    /// session ends or the core restarts.
    ApproveAlwaysForTool,
    /// Run the call AND trust this exact `(flow_id, tool_name)` pair for the
    /// flow the pending row's `source_context` names — see
    /// [`ApprovalSourceContext::Flow`]. Unlike [`Self::ApproveAlwaysForTool`]
    /// (a global, cross-flow allowlist), this grant is scoped to one
    /// workflow: subsequent runs of *that* flow (including scheduled/
    /// triggered ones) auto-allow *that* tool without prompting, but a new
    /// tool the flow starts calling still parks. Persisted via
    /// `approval::store::insert_flow_trust`, not `autonomy.auto_approve`.
    /// Only meaningful when `source_context` is `Some(Flow { .. })` — the
    /// `approval_decide` RPC handler logs and no-ops the persistence step
    /// otherwise (the decision itself still resolves the parked call).
    ApproveAlwaysForFlow,
    /// Reject the call. The agent receives a structured error string.
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ApproveAlwaysForTool => "approve_always_for_tool",
            Self::ApproveAlwaysForFlow => "approve_always_for_flow",
            Self::Deny => "deny",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "approve_once" => Some(Self::ApproveOnce),
            "approve_always_for_tool" => Some(Self::ApproveAlwaysForTool),
            "approve_always_for_flow" => Some(Self::ApproveAlwaysForFlow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn is_approve(self) -> bool {
        matches!(
            self,
            Self::ApproveOnce | Self::ApproveAlwaysForTool | Self::ApproveAlwaysForFlow
        )
    }
}

/// Outcome of routing a tool call through `ApprovalGate::intercept`.
#[derive(Debug, Clone)]
pub enum GateOutcome {
    /// Proceed with `tool.execute(args)`.
    Allow,
    /// Abort the call. The agent sees `reason` in place of a tool
    /// result.
    Deny { reason: String },
}

/// Terminal status of a tool action that the gate previously allowed.
///
/// Recorded after the tool finishes so the audit row in
/// `pending_approvals` carries a full before-and-after trail per the
/// issue #2135 acceptance criterion. The variant set is intentionally
/// small — anything richer belongs in the structured tool result,
/// not the approval audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    /// Tool ran and returned a non-error [`ToolResult`].
    Success,
    /// Tool ran and returned an error [`ToolResult`] (or panicked).
    Failure,
    /// Tool did not run because the runtime aborted (timeout,
    /// cancellation, supervisor shutdown).
    Aborted,
}

impl ExecutionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Aborted => "aborted",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_decision_round_trips() {
        for d in [
            ApprovalDecision::ApproveOnce,
            ApprovalDecision::ApproveAlwaysForTool,
            ApprovalDecision::ApproveAlwaysForFlow,
            ApprovalDecision::Deny,
        ] {
            assert_eq!(ApprovalDecision::from_str(d.as_str()), Some(d));
        }
    }

    #[test]
    fn from_str_unknown_decision_is_none() {
        assert!(ApprovalDecision::from_str("maybe").is_none());
    }

    #[test]
    fn is_approve_true_for_approval_variants_only() {
        assert!(ApprovalDecision::ApproveOnce.is_approve());
        assert!(ApprovalDecision::ApproveAlwaysForTool.is_approve());
        assert!(ApprovalDecision::ApproveAlwaysForFlow.is_approve());
        assert!(!ApprovalDecision::Deny.is_approve());
    }

    #[test]
    fn approve_always_for_flow_serializes_as_snake_case() {
        let s = serde_json::to_string(&ApprovalDecision::ApproveAlwaysForFlow).unwrap();
        assert_eq!(s, "\"approve_always_for_flow\"");
    }

    #[test]
    fn source_context_flow_round_trips_as_internally_tagged_json() {
        let ctx = ApprovalSourceContext::Flow {
            flow_id: "flow-1".to_string(),
            run_id: "run-1".to_string(),
            node_id: None,
        };
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["kind"], "flow");
        assert_eq!(json["flow_id"], "flow-1");
        assert_eq!(json["run_id"], "run-1");
        assert!(
            json.get("node_id").is_none(),
            "None node_id must be omitted, not null"
        );

        let back: ApprovalSourceContext = serde_json::from_value(json).unwrap();
        assert_eq!(back, ctx);
    }

    #[test]
    fn pending_approval_source_context_defaults_to_none_and_is_omitted_when_absent() {
        let p = PendingApproval::new(
            "req-1",
            "composio",
            "send email",
            serde_json::json!({}),
            None,
        );
        assert!(p.source_context.is_none());
        let json = serde_json::to_value(&p).unwrap();
        assert!(
            json.get("source_context").is_none(),
            "absent source_context must not be serialized as null: {json}"
        );
    }

    #[test]
    fn with_source_context_attaches_flow_context() {
        let p = PendingApproval::new(
            "req-1",
            "composio",
            "send email",
            serde_json::json!({}),
            None,
        )
        .with_source_context(ApprovalSourceContext::Flow {
            flow_id: "flow-1".to_string(),
            run_id: "run-1".to_string(),
            node_id: Some("node-1".to_string()),
        });
        match p.source_context {
            Some(ApprovalSourceContext::Flow {
                flow_id,
                run_id,
                node_id,
            }) => {
                assert_eq!(flow_id, "flow-1");
                assert_eq!(run_id, "run-1");
                assert_eq!(node_id.as_deref(), Some("node-1"));
            }
            other => panic!("expected Flow source_context, got {other:?}"),
        }
    }

    #[test]
    fn approval_decision_serializes_as_snake_case() {
        let s = serde_json::to_string(&ApprovalDecision::ApproveAlwaysForTool).unwrap();
        assert_eq!(s, "\"approve_always_for_tool\"");
    }

    #[test]
    fn execution_outcome_round_trips() {
        for o in [
            ExecutionOutcome::Success,
            ExecutionOutcome::Failure,
            ExecutionOutcome::Aborted,
        ] {
            assert_eq!(ExecutionOutcome::from_str(o.as_str()), Some(o));
        }
        assert!(ExecutionOutcome::from_str("partial").is_none());
    }

    #[test]
    fn execution_outcome_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ExecutionOutcome::Success).unwrap(),
            "\"success\""
        );
        assert_eq!(
            serde_json::to_string(&ExecutionOutcome::Aborted).unwrap(),
            "\"aborted\""
        );
    }

    /// Regression guard. Earlier revisions of [`PendingApproval`]
    /// exposed a `session_id: String` field — when an operator had
    /// set the RPC bearer to a stable value, that field carried the
    /// raw credential, and Debug-formatting / serializing a pending
    /// row was enough to leak it. Both surfaces are exercised here.
    #[test]
    fn pending_approval_debug_and_serialize_do_not_carry_session_id() {
        let p = PendingApproval {
            request_id: "req-1".to_string(),
            tool_name: "composio".to_string(),
            action_summary: "send slack message".to_string(),
            args_redacted: serde_json::json!({ "tool_slug": "SLACK_SEND" }),
            created_at: Utc::now(),
            expires_at: None,
            source_context: None,
        };
        let dbg = format!("{p:?}");
        assert!(
            !dbg.contains("session_id"),
            "Debug output must not surface session_id: {dbg}"
        );
        let json = serde_json::to_value(&p).unwrap();
        assert!(
            json.get("session_id").is_none(),
            "Serialized JSON must not surface session_id: {json}"
        );

        let audit = ApprovalAuditEntry {
            request_id: "req-1".to_string(),
            tool_name: "composio".to_string(),
            action_summary: "send slack message".to_string(),
            args_redacted: serde_json::json!({ "tool_slug": "SLACK_SEND" }),
            created_at: Utc::now(),
            expires_at: None,
            decided_at: Utc::now(),
            decision: ApprovalDecision::ApproveOnce,
        };
        let audit_dbg = format!("{audit:?}");
        assert!(
            !audit_dbg.contains("session_id"),
            "ApprovalAuditEntry Debug must not surface session_id: {audit_dbg}"
        );
        let audit_json = serde_json::to_value(&audit).unwrap();
        assert!(
            audit_json.get("session_id").is_none(),
            "ApprovalAuditEntry JSON must not surface session_id: {audit_json}"
        );
    }
}
