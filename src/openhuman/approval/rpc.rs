//! Approval RPC operations.
//!
//! Exposed as `approval_list_pending` and `approval_decide` through
//! the controller registry (see [`super::schemas`]).

use anyhow::anyhow;

use crate::rpc::RpcOutcome;

use super::gate::{try_boot_state, ApprovalGate, ApprovalGateBootState, DecideMiss};
use super::types::{ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, PendingApproval};

/// Read the host-aware approval-gate boot decision so the UI banner can
/// render the right state on first paint (rather than waiting for a
/// connected socket subscriber to catch a transient boot-time event).
///
/// Returns a benign "installed, no banner" default when the boot state was
/// never recorded — older test paths that bring up the gate directly bypass
/// `bootstrap_core_runtime` and therefore never call `record_boot_state`.
pub async fn approval_get_gate_state() -> anyhow::Result<RpcOutcome<ApprovalGateBootState>> {
    tracing::debug!("[rpc:approval_get_gate_state] entry");
    let state = try_boot_state().unwrap_or(ApprovalGateBootState {
        installed: ApprovalGate::try_global().is_some(),
        disabled_by_env: false,
        override_ignored: false,
        host: "unknown",
    });
    tracing::debug!(
        installed = state.installed,
        disabled_by_env = state.disabled_by_env,
        override_ignored = state.override_ignored,
        host = state.host,
        "[rpc:approval_get_gate_state] exit"
    );
    Ok(RpcOutcome::new(state, vec![]))
}

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
    let row = match decided {
        Some(row) => row,
        None => {
            // `gate.decide` returned `Ok(None)`: the conditional UPDATE matched
            // 0 rows. Disambiguate the benign already-decided/expired race from
            // a genuine lost registration so only the latter reaches Sentry
            // (TAURI-RUST-5EH). Both stay `warn!` — neither is a hard error the
            // caller can act on differently — but the benign arm emits the
            // "no pending approval found" wording that `expected_error_kind`
            // demotes, while the never-registered arm emits a distinct string
            // that stays a captured Sentry signal.
            return match gate.classify_decide_miss(request_id) {
                DecideMiss::AlreadyResolved => {
                    tracing::warn!(
                        request_id = request_id,
                        "[rpc:approval_decide] no pending approval found (already decided/expired — benign race)"
                    );
                    Err(anyhow!(
                        "no pending approval found for request_id '{request_id}' (already decided or expired)"
                    ))
                }
                DecideMiss::NeverRegistered => {
                    tracing::warn!(
                        request_id = request_id,
                        "[rpc:approval_decide] no pending approval ever registered (genuine lost registration)"
                    );
                    Err(anyhow!(
                        "no pending approval ever registered for request_id '{request_id}'"
                    ))
                }
            };
        }
    };

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

    // "Approve always for this flow" (flow-approval-surface, PR2): unlike
    // `ApproveAlwaysForTool` above, this grant is scoped to one workflow —
    // persisted into `flow_tool_trust`, NOT the global `autonomy.auto_approve`
    // allowlist, so it cannot leak into a different flow that happens to call
    // the same tool. Only meaningful on a row whose `source_context` names a
    // flow; a row without one (decision picked on a non-flow park, which the
    // UI should never offer but the RPC must not trust blindly) logs a
    // warning and no-ops the persistence — the decision itself still
    // resolved the parked call above, so the RPC does not fail.
    if decision == ApprovalDecision::ApproveAlwaysForFlow {
        match &row.source_context {
            Some(ApprovalSourceContext::Flow { flow_id, .. }) => {
                match gate.insert_flow_trust(flow_id, &row.tool_name) {
                    Ok(()) => {
                        tracing::info!(
                            flow_id = flow_id.as_str(),
                            tool = row.tool_name.as_str(),
                            "[rpc:approval_decide] tool persisted to flow_tool_trust"
                        );
                        logs.push(format!(
                            "[approval] '{}' added to '{}'s Always-allow list",
                            row.tool_name, flow_id
                        ));
                    }
                    Err(err) => {
                        tracing::warn!(
                            flow_id = flow_id.as_str(),
                            tool = row.tool_name.as_str(),
                            error = %err,
                            "[rpc:approval_decide] failed to persist flow_tool_trust; tool will prompt again next run"
                        );
                        logs.push(format!(
                            "[approval] WARNING: could not save 'Always allow' for '{}' on flow '{flow_id}': {err}",
                            row.tool_name
                        ));
                    }
                }
            }
            None => {
                tracing::warn!(
                    request_id = row.request_id.as_str(),
                    tool = row.tool_name.as_str(),
                    "[rpc:approval_decide] ApproveAlwaysForFlow decided a row with no flow \
                     source_context — nothing to persist, decision still resolved"
                );
                logs.push(format!(
                    "[approval] WARNING: '{}' has no flow context — 'Always allow for this workflow' \
                     was not persisted",
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
