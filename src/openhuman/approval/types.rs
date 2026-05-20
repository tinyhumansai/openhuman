//! Types shared between the `ApprovalGate`, the SQLite store, and the
//! RPC layer. Kept narrow so the gate, the store, and the RPC ops can
//! evolve independently without circular imports through `mod.rs`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A tool call that has been intercepted and is awaiting a user
/// decision. Persisted in `pending_approvals` and surfaced to the UI
/// via `approval_list_pending`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub request_id: String,
    pub tool_name: String,
    /// Short human-readable summary (scrubbed of PII / chat content
    /// per `feedback_redact_paths_and_ids_in_public.md`).
    pub action_summary: String,
    /// Redacted JSON arguments — counts/shape only, no raw message
    /// bodies, per `feedback_pr_no_chat_content.md`.
    pub args_redacted: serde_json::Value,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
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
}
