//! Approval RPC operations.
//!
//! Exposed as `approval_list_pending` and `approval_decide` through
//! the controller registry (see [`super::schemas`]).

use anyhow::anyhow;

use crate::rpc::RpcOutcome;

use super::gate::ApprovalGate;
use super::types::{ApprovalDecision, PendingApproval};

/// List rows still awaiting a user decision in the current session.
///
/// Returns an empty list (not an error) when the gate is not
/// installed — supervised mode may be disabled, in which case there
/// is nothing pending by definition.
pub async fn approval_list_pending() -> anyhow::Result<RpcOutcome<Vec<PendingApproval>>> {
    let Some(gate) = ApprovalGate::try_global() else {
        return Ok(RpcOutcome::new(Vec::new(), vec![]));
    };
    let rows = gate.list_pending()?;
    let log = format!("[approval] list_pending returned {} row(s)", rows.len());
    Ok(RpcOutcome::single_log(rows, log))
}

/// Apply a decision to a pending row. Errors when the request id is
/// unknown / already decided / belongs to a different session.
pub async fn approval_decide(
    request_id: &str,
    decision: ApprovalDecision,
) -> anyhow::Result<RpcOutcome<PendingApproval>> {
    let gate = ApprovalGate::try_global()
        .ok_or_else(|| anyhow!("approval gate is not installed; supervised mode disabled"))?;
    let decided = gate.decide(request_id, decision)?;
    let row = decided
        .ok_or_else(|| anyhow!("no pending approval found for request_id '{request_id}'"))?;
    let log = format!(
        "[approval] decided request_id={} tool={} decision={}",
        row.request_id,
        row.tool_name,
        decision.as_str()
    );
    Ok(RpcOutcome::single_log(row, log))
}
