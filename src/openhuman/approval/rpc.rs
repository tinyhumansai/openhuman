//! Approval RPC operations.
//!
//! Exposed as `approval_list_pending` and `approval_decide` through
//! the controller registry (see [`super::schemas`]).

use anyhow::anyhow;

use crate::rpc::RpcOutcome;

use super::gate::ApprovalGate;
use super::types::{ApprovalAuditEntry, ApprovalDecision, PendingApproval};

/// List rows still awaiting a user decision in the current session.
///
/// Returns an empty list (not an error) when the gate is not
/// installed — supervised mode may be disabled, in which case there
/// is nothing pending by definition.
pub async fn approval_list_pending() -> anyhow::Result<RpcOutcome<Vec<PendingApproval>>> {
    tracing::debug!("[rpc:approval_list_pending] entry");
    let Some(gate) = ApprovalGate::try_global() else {
        tracing::debug!("[rpc:approval_list_pending] gate not installed, returning empty");
        return Ok(RpcOutcome::new(Vec::new(), vec![]));
    };
    let rows = match gate.list_pending() {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "[rpc:approval_list_pending] store error");
            return Err(err);
        }
    };
    tracing::debug!(rows = rows.len(), "[rpc:approval_list_pending] exit");
    let log = format!("[approval] list_pending returned {} row(s)", rows.len());
    Ok(RpcOutcome::single_log(rows, log))
}

/// List recently decided approval rows for audit/diagnostic surfaces.
pub async fn approval_list_recent_decisions(
    limit: Option<usize>,
) -> anyhow::Result<RpcOutcome<Vec<ApprovalAuditEntry>>> {
    tracing::debug!("[rpc:approval_list_recent_decisions] entry");
    let Some(gate) = ApprovalGate::try_global() else {
        tracing::debug!("[rpc:approval_list_recent_decisions] gate not installed, returning empty");
        return Ok(RpcOutcome::new(Vec::new(), vec![]));
    };
    let limit = limit.unwrap_or(50);
    let rows = match gate.list_recent_decisions(limit) {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "[rpc:approval_list_recent_decisions] store error");
            return Err(err);
        }
    };
    let log = format!(
        "[approval] list_recent_decisions returned {} row(s)",
        rows.len()
    );
    tracing::debug!(
        rows = rows.len(),
        limit = limit,
        "[rpc:approval_list_recent_decisions] exit"
    );
    Ok(RpcOutcome::single_log(rows, log))
}

/// Apply a decision to a pending row. Errors when the request id is
/// unknown / already decided / belongs to a different session.
pub async fn approval_decide(
    request_id: &str,
    decision: ApprovalDecision,
) -> anyhow::Result<RpcOutcome<PendingApproval>> {
    tracing::debug!(
        request_id = request_id,
        decision = decision.as_str(),
        "[rpc:approval_decide] entry"
    );
    let gate = ApprovalGate::try_global().ok_or_else(|| {
        tracing::warn!(
            request_id = request_id,
            "[rpc:approval_decide] gate not installed"
        );
        anyhow!("approval gate is not installed; supervised mode disabled")
    })?;
    let decided = match gate.decide(request_id, decision) {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(
                request_id = request_id,
                error = %err,
                "[rpc:approval_decide] gate decide failed"
            );
            return Err(err);
        }
    };
    let row = decided.ok_or_else(|| {
        tracing::warn!(
            request_id = request_id,
            "[rpc:approval_decide] no pending approval found"
        );
        anyhow!("no pending approval found for request_id '{request_id}'")
    })?;

    let mut logs = vec![format!(
        "[approval] decided request_id={} tool={} decision={}",
        row.request_id,
        row.tool_name,
        decision.as_str()
    )];

    // "Always allow": persist the tool onto the user's `autonomy.auto_approve`
    // allowlist (config save + live-policy reload) so the gate skips prompting
    // for it on future turns — this session and across restarts. Best-effort:
    // `gate.decide` already resolved the current call, so a persistence failure
    // must not fail the RPC. It degrades safely — the tool simply prompts again
    // next time rather than being silently auto-approved.
    if decision == ApprovalDecision::ApproveAlwaysForTool {
        match crate::openhuman::config::ops::add_auto_approve_tool(&row.tool_name).await {
            Ok(()) => {
                tracing::info!(
                    tool = row.tool_name.as_str(),
                    "[rpc:approval_decide] tool persisted to auto_approve allowlist"
                );
                logs.push(format!(
                    "[approval] '{}' added to the Always-allow list",
                    row.tool_name
                ));
            }
            Err(err) => {
                tracing::warn!(
                    tool = row.tool_name.as_str(),
                    error = %err,
                    "[rpc:approval_decide] failed to persist auto_approve; tool will prompt again next time"
                );
                logs.push(format!(
                    "[approval] WARNING: could not save 'Always allow' for '{}': {err}",
                    row.tool_name
                ));
            }
        }
    }

    tracing::info!(
        request_id = row.request_id.as_str(),
        tool = row.tool_name.as_str(),
        decision = decision.as_str(),
        "[rpc:approval_decide] exit"
    );
    Ok(RpcOutcome::new(row, logs))
}
