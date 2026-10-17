use super::*;
use crate::security::approval::types::{ApprovalDecision, PendingApproval};
use chrono::{DateTime, Duration, Utc};
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
        tool_call_id: None,
        agent_id: None,
    }
}

/// The stored document of `request_id`, as the store wrote it.
fn raw_doc(config: &Config, request_id: &str) -> Option<serde_json::Value> {
    let opened = crate::storage::local::open("approval::store-test", &config.workspace_dir)
        .expect("open the storage backend");
    let docs = std::sync::Arc::clone(opened.scoped().expect("scope").documents());
    let id = request_id.to_string();
    crate::storage::block_on(async move { docs.get("approvals.requests", &id).await })
        .expect("read the approval document")
        .map(|stored| stored.doc)
}

fn text_field(doc: &serde_json::Value, field: &str) -> Option<String> {
    doc.get(field).and_then(|v| v.as_str()).map(str::to_string)
}

/// An instant field (epoch milliseconds) as text, so a test asks "is it set".
fn time_field(doc: &serde_json::Value, field: &str) -> Option<String> {
    doc.get(field)
        .and_then(serde_json::Value::as_i64)
        .map(|ms| ms.to_string())
}

fn fetch_decision_state(
    config: &Config,
    request_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    raw_doc(config, request_id)
        .map(|doc| (time_field(&doc, "decided_ms"), text_field(&doc, "decision")))
}

// ── record_execution tests (#2135) ──────────

fn read_execution_row(
    config: &Config,
    request_id: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let doc = raw_doc(config, request_id).expect("row exists");
    (
        time_field(&doc, "executed_ms"),
        text_field(&doc, "execution_outcome"),
        text_field(&doc, "execution_error"),
    )
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

#[path = "store_flow_trust_tests.rs"]
mod flow_trust_tests;
#[path = "store_persistence_tests.rs"]
mod persistence_tests;
