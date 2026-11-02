//! The history view: approvals the module decided, read from the stored rows.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use tinysecurity_bus::{ApprovalChoice2, ApprovalLifecycle2, ApprovalRow2, JudgeEvidence2};

use super::super::types::{ApprovalAuditEntry, ApprovalDecision, JudgeVerdict};
use super::rows::{Rows, StoredRow};
use crate::config::Config;

fn instant(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

fn judge(evidence: &JudgeEvidence2) -> JudgeVerdict {
    let millis = |value: u32| u16::try_from(value / 1000).ok();
    JudgeVerdict {
        outcome: format!("{:?}", evidence.outcome).to_lowercase(),
        intent_millis: evidence.scores_millionths.and_then(|s| millis(s[0])),
        reversible_millis: evidence.scores_millionths.and_then(|s| millis(s[1])),
        exfiltrate_millis: evidence.scores_millionths.and_then(|s| millis(s[2])),
        cached: evidence.cached,
    }
}

/// What the user chose, in the host's words. A row the module approved without a
/// human was approved once; the label the host kept distinguishes "always allow".
fn decision(row: &ApprovalRow2, label: Option<&str>) -> ApprovalDecision {
    match row.state {
        ApprovalLifecycle2::Approved
        | ApprovalLifecycle2::Reserved
        | ApprovalLifecycle2::Executed
            if row.choice != Some(ApprovalChoice2::Deny) =>
        {
            label
                .and_then(ApprovalDecision::from_str)
                .filter(|chosen| chosen.is_approve())
                .unwrap_or(ApprovalDecision::ApproveOnce)
        }
        _ => ApprovalDecision::Deny,
    }
}

/// Who decided, when it was not a person: the module's `auto:*` attributions.
fn decided_by(row: &ApprovalRow2) -> Option<String> {
    row.attribution
        .as_deref()
        .filter(|who| who.starts_with("auto:"))
        .map(str::to_owned)
}

/// Attribution of a durable flow-trust row (`GrantFlowTool2`).
const FLOW_TRUST: &str = "flow-trust-grant";

/// The summary a flow pre-authorization has always shown in the history.
const FLOW_TRUST_SUMMARY: &str = "Pre-authorized for this flow when it was saved and enabled";

/// What an imported legacy request was decided as, from the record the module
/// kept (`legacy_history_json`).
fn imported_decision(row: &ApprovalRow2) -> Option<ApprovalDecision> {
    let json = row.legacy_history_json.as_deref()?;
    let record: Value = serde_json::from_str(json).ok()?;
    Some(match record.get("state").and_then(Value::as_str) {
        Some("allowed" | "executed") => ApprovalDecision::ApproveOnce,
        _ => ApprovalDecision::Deny,
    })
}

fn entry(stored: &StoredRow, label: Option<&str>) -> Option<ApprovalAuditEntry> {
    let row = &stored.row;
    let decided_at = instant(stored.decided_ms?);
    if row.attribution.as_deref() == Some(FLOW_TRUST) {
        // A grant is history while it stands; a revoked one is not a decision.
        if row.state != ApprovalLifecycle2::Approved {
            return None;
        }
        return Some(ApprovalAuditEntry {
            request_id: row.request_id.clone(),
            tool_name: row.tool_name.clone(),
            action_summary: FLOW_TRUST_SUMMARY.to_owned(),
            args_redacted: Value::Null,
            created_at: instant(i64::try_from(row.created_at_ms).unwrap_or(i64::MAX)),
            expires_at: None,
            decided_at,
            decision: ApprovalDecision::ApproveAlwaysForFlow,
            decided_by: None,
            judge: None,
        });
    }
    if let Some(decision) = imported_decision(row) {
        return Some(ApprovalAuditEntry {
            request_id: row.request_id.clone(),
            tool_name: "earlier approval".to_owned(),
            action_summary: "Recorded by an earlier version".to_owned(),
            args_redacted: Value::Null,
            created_at: instant(i64::try_from(row.created_at_ms).unwrap_or(i64::MAX)),
            expires_at: None,
            decided_at,
            decision,
            decided_by: None,
            judge: None,
        });
    }
    Some(ApprovalAuditEntry {
        request_id: row.request_id.clone(),
        tool_name: row.tool_name.clone(),
        action_summary: row.summary.clone(),
        args_redacted: serde_json::from_str::<Value>(&row.args_redacted).unwrap_or(Value::Null),
        created_at: instant(i64::try_from(row.created_at_ms).unwrap_or(i64::MAX)),
        expires_at: Some(instant(
            i64::try_from(row.expires_at_ms).unwrap_or(i64::MAX),
        )),
        decided_at,
        decision: decision(row, label),
        decided_by: decided_by(row),
        judge: row.judge.as_ref().map(judge),
    })
}

/// The most recently decided approvals of the current scope, newest first.
pub(crate) async fn decided_history(
    config: &Config,
    limit: usize,
) -> anyhow::Result<Vec<ApprovalAuditEntry>> {
    let rows = Rows::current(config)?;
    let stored = rows
        .decided(limit)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    let mut entries = Vec::with_capacity(stored.len());
    for stored in &stored {
        let label = rows.label(&stored.row.request_id).await.ok().flatten();
        entries.extend(entry(stored, label.as_deref()));
    }
    Ok(entries)
}

/// Whether the request was stored and has left `Pending`. `None` when it is not
/// stored at all.
pub(crate) async fn row_decided(config: &Config, request_id: &str) -> anyhow::Result<Option<bool>> {
    let rows = Rows::current(config)?;
    let stored = rows
        .row(request_id)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok(stored.map(|stored| stored.row.state != ApprovalLifecycle2::Pending))
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
