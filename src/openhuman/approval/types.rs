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
    /// Reject the call. The agent receives a structured error string.
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ApproveAlwaysForTool => "approve_always_for_tool",
            Self::Deny => "deny",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "approve_once" => Some(Self::ApproveOnce),
            "approve_always_for_tool" => Some(Self::ApproveAlwaysForTool),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn is_approve(self) -> bool {
        matches!(self, Self::ApproveOnce | Self::ApproveAlwaysForTool)
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
        assert!(!ApprovalDecision::Deny.is_approve());
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
