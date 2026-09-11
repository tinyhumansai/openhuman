use super::*;
use crate::openhuman::security::approval::types::{ApprovalDecision, PendingApproval};
use chrono::Duration;
use serde_json::json;
use tempfile::TempDir;

fn test_config() -> (Config, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (config, dir)
}

/// Build a sample `PendingApproval`. The `_session_id` parameter
/// is preserved as a positional argument for call-site readability
/// (so a reader can see "this row belongs to sess-A") even though
/// it is no longer stamped onto [`PendingApproval`]; the call site
/// passes it through to [`insert_pending`] as the third argument.
fn sample(request_id: &str, _session_id: &str) -> PendingApproval {
    sample_with_expiry(
        request_id,
        _session_id,
        Some(Utc::now() + Duration::minutes(10)),
    )
}

fn sample_with_expiry(
    request_id: &str,
    _session_id: &str,
    expires_at: Option<DateTime<Utc>>,
) -> PendingApproval {
    PendingApproval {
        request_id: request_id.to_string(),
        tool_name: "composio".to_string(),
        action_summary: "send slack message (12 chars)".to_string(),
        args_redacted: json!({ "action": "execute", "tool_slug": "SLACK_SEND" }),
        created_at: Utc::now(),
        expires_at,
        source_context: None,
    }
}

fn fetch_decision_state(
    config: &Config,
    request_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare("SELECT decided_at, decision FROM pending_approvals WHERE request_id = ?1")
            .context("prepare raw decision lookup")?;
        let mut rows = stmt
            .query(params![request_id])
            .context("query raw decision lookup")?;
        if let Some(row) = rows.next().context("decision row next")? {
            let decided_at: Option<String> = row.get(0)?;
            let decision: Option<String> = row.get(1)?;
            Ok(Some((decided_at, decision)))
        } else {
            Ok(None)
        }
    })
    .unwrap()
}

// ── record_execution / column-migration tests (#2135) ──────────

fn read_execution_row(
    config: &Config,
    request_id: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT executed_at, execution_outcome, execution_error
                 FROM pending_approvals WHERE request_id = ?1",
            )
            .unwrap();
        let mut rows = stmt.query(params![request_id]).unwrap();
        let row = rows.next().unwrap().expect("row exists");
        Ok((
            row.get::<_, Option<String>>(0).unwrap(),
            row.get::<_, Option<String>>(1).unwrap(),
            row.get::<_, Option<String>>(2).unwrap(),
        ))
    })
    .unwrap()
}

// ── source_context / flow_tool_trust (flow-approval-surface, PR2) ──────

fn flow_sample(request_id: &str, flow_id: &str, run_id: &str) -> PendingApproval {
    PendingApproval::new(
        request_id,
        "composio",
        "send slack message",
        json!({ "action": "execute" }),
        Some(Utc::now() + Duration::minutes(10)),
    )
    .with_source_context(ApprovalSourceContext::Flow {
        flow_id: flow_id.to_string(),
        run_id: run_id.to_string(),
        node_id: None,
    })
}

#[path = "store_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "store_tests_part_02_tests.rs"]
mod part_02_tests;
