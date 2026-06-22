//! Tests for the task-recency window narrowing + post-filter.

use super::*;
use chrono::TimeZone;
use serde_json::json;

fn floor() -> DateTime<Utc> {
    // Fixed "now - 24h" floor for deterministic comparisons.
    Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap()
}

fn ok_resp(data: Value) -> ComposioExecuteResponse {
    ComposioExecuteResponse {
        data,
        successful: true,
        error: None,
        cost_usd: 0.0,
        markdown_formatted: None,
    }
}

// ── post-filter: ClickUp (epoch-millis string) ──────────────────────

#[test]
fn clickup_drops_rows_older_than_floor() {
    let old_ms = (floor().timestamp_millis() - 1).to_string();
    let new_ms = (floor().timestamp_millis() + 60_000).to_string();
    let resp = ok_resp(json!({
        "data": { "tasks": [
            { "id": "old", "date_updated": old_ms },
            { "id": "new", "date_updated": new_ms },
        ]}
    }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    let tasks = out.data["data"]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], "new");
}

#[test]
fn clickup_keeps_row_exactly_on_floor() {
    let on_ms = floor().timestamp_millis().to_string();
    let resp = ok_resp(json!({ "tasks": [{ "id": "edge", "date_updated": on_ms }] }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert_eq!(out.data["tasks"].as_array().unwrap().len(), 1);
}

// ── post-filter: Linear (ISO-8601, GraphQL `nodes` envelope) ─────────

#[test]
fn linear_filters_iso_timestamps_under_nodes() {
    let resp = ok_resp(json!({
        "nodes": [
            { "id": "old", "updatedAt": "2026-06-19T09:00:00.000Z" },
            { "id": "new", "updatedAt": "2026-06-20T12:00:00.000Z" },
        ]
    }));
    let out = filter_response("LINEAR_LIST_LINEAR_ISSUES", resp, floor());
    let nodes = out.data["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["id"], "new");
}

#[test]
fn linear_filters_graphql_connection_under_data_issues_nodes() {
    // Real Linear/Composio shape: { data: { issues: { nodes: [...] } } }.
    // Regression for the no-array pass-through that left the backlog unfiltered.
    let resp = ok_resp(json!({
        "data": { "issues": { "nodes": [
            { "id": "old", "updatedAt": "2026-06-19T09:00:00.000Z" },
            { "id": "new", "updatedAt": "2026-06-20T12:00:00.000Z" },
        ]}}
    }));
    let out = filter_response("LINEAR_LIST_LINEAR_ISSUES", resp, floor());
    let nodes = out.data["data"]["issues"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["id"], "new");
}

#[test]
fn filters_data_wrapped_row_timestamps() {
    // Composio sometimes wraps each row under `data` — the timestamp must
    // still be read (else stale rows survive as "no timestamp → keep").
    let resp = ok_resp(json!({
        "tasks": [
            { "data": { "id": "old", "date_updated": "0" } },
            { "data": { "id": "new",
                "date_updated": (floor().timestamp_millis()+1).to_string() } },
        ]
    }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    let tasks = out.data["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["data"]["id"], "new");
}

// ── post-filter: Notion (keep if EITHER timestamp is fresh) ──────────

#[test]
fn notion_keeps_page_when_either_timestamp_fresh() {
    let resp = ok_resp(json!({
        "results": [
            // created long ago but recently edited → keep
            { "id": "edited", "created_time": "2026-01-01T00:00:00.000Z",
              "last_edited_time": "2026-06-20T10:00:00.000Z" },
            // both stale → drop
            { "id": "stale", "created_time": "2026-01-01T00:00:00.000Z",
              "last_edited_time": "2026-01-02T00:00:00.000Z" },
        ]
    }));
    let out = filter_response("NOTION_QUERY_DATABASE_WITH_FILTER", resp, floor());
    let results = out.data["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], "edited");
}

// ── post-filter: Todoist (added_at / updated_at) ────────────────────

#[test]
fn todoist_drops_tasks_added_before_floor() {
    // Real Todoist v1 field is `added_at` (not created_at).
    let resp = ok_resp(json!({
        "tasks": [
            { "id": "old", "added_at": "2026-06-19T09:00:00.000Z" },
            { "id": "new", "added_at": "2026-06-20T12:00:00.000Z" },
        ]
    }));
    let out = filter_response("TODOIST_GET_ALL_TASKS", resp, floor());
    let tasks = out.data["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], "new");
}

#[test]
fn todoist_keeps_task_updated_in_window_even_if_added_earlier() {
    // created-or-modified: added long ago but updated recently → keep.
    let resp = ok_resp(json!({
        "tasks": [
            { "id": "touched", "added_at": "2026-01-01T00:00:00.000Z",
              "updated_at": "2026-06-20T12:00:00.000Z" },
            { "id": "stale", "added_at": "2026-01-01T00:00:00.000Z",
              "updated_at": "2026-01-02T00:00:00.000Z" },
        ]
    }));
    let out = filter_response("TODOIST_GET_ALL_TASKS", resp, floor());
    let tasks = out.data["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], "touched");
}

#[test]
fn todoist_handles_bare_array_and_data_wrapped_rows() {
    // Composio may return a bare array and/or wrap each row under `data`.
    let resp = ok_resp(json!([
        { "data": { "id": "old", "added_at": "2026-01-01T00:00:00.000Z" } },
        { "data": { "id": "new", "added_at": "2026-06-20T12:00:00.000Z" } },
    ]));
    let out = filter_response("TODOIST_GET_ALL_TASKS", resp, floor());
    let tasks = out.data.as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["data"]["id"], "new");
}

// ── conservative behaviors ──────────────────────────────────────────

#[test]
fn rows_without_timestamp_are_kept() {
    let resp = ok_resp(json!({ "tasks": [{ "id": "no_ts" }] }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert_eq!(out.data["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn unparseable_timestamp_is_kept() {
    let resp = ok_resp(json!({ "tasks": [{ "id": "junk", "date_updated": "not-a-number" }] }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert_eq!(out.data["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn unknown_slug_is_untouched() {
    let resp = ok_resp(json!({ "tasks": [{ "id": "x", "date_updated": "0" }] }));
    let out = filter_response("GMAIL_FETCH_EMAILS", resp, floor());
    assert_eq!(out.data["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn failed_response_is_untouched() {
    let old_ms = "0".to_string();
    let mut resp = ok_resp(json!({ "tasks": [{ "id": "old", "date_updated": old_ms }] }));
    resp.successful = false;
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert_eq!(out.data["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn missing_items_array_is_pass_through() {
    let resp = ok_resp(json!({ "unexpected": "shape" }));
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert_eq!(out.data["unexpected"], "shape");
}

// ── markdown_formatted invalidation ─────────────────────────────────

#[test]
fn markdown_cleared_only_when_rows_removed() {
    // Rows removed → markdown dropped so agent reads filtered JSON.
    let mut resp = ok_resp(json!({ "tasks": [
        { "id": "old", "date_updated": "0" },
        { "id": "new", "date_updated": (floor().timestamp_millis()+1).to_string() },
    ]}));
    resp.markdown_formatted = Some("| old | new | full backlog |".into());
    let out = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp, floor());
    assert!(out.markdown_formatted.is_none());

    // Nothing removed → markdown preserved.
    let mut resp2 = ok_resp(json!({ "tasks": [
        { "id": "new", "date_updated": (floor().timestamp_millis()+1).to_string() },
    ]}));
    resp2.markdown_formatted = Some("kept".into());
    let out2 = filter_response("CLICKUP_GET_FILTERED_TEAM_TASKS", resp2, floor());
    assert_eq!(out2.markdown_formatted.as_deref(), Some("kept"));
}

// ── apply_window_args ───────────────────────────────────────────────

#[test]
fn injects_order_arg_when_absent() {
    let out = apply_window_args("LINEAR_LIST_LINEAR_ISSUES", None, floor()).unwrap();
    assert_eq!(out["orderBy"], "updatedAt");
}

#[test]
fn caller_supplied_order_arg_wins() {
    let args = json!({ "order_by": "created" });
    let out = apply_window_args("CLICKUP_GET_FILTERED_TEAM_TASKS", Some(args), floor()).unwrap();
    assert_eq!(out["order_by"], "created");
}

#[test]
fn asana_gets_modified_since_injected() {
    let out = apply_window_args("ASANA_GET_MULTIPLE_TASKS", Some(json!({})), floor()).unwrap();
    assert_eq!(out["modified_since"], floor().to_rfc3339());
}

#[test]
fn apply_window_args_noop_for_unknown_slug() {
    let out = apply_window_args("GMAIL_FETCH_EMAILS", Some(json!({ "a": 1 })), floor());
    assert_eq!(out.unwrap(), json!({ "a": 1 }));
}

#[test]
fn todoist_gets_no_server_side_narrowing() {
    // GET /tasks has no `filter` param (that's the /tasks/filter endpoint),
    // so we must NOT inject one — enforcement is pure post-filter.
    let out = apply_window_args(
        "TODOIST_GET_ALL_TASKS",
        Some(json!({ "limit": 50 })),
        floor(),
    )
    .unwrap();
    assert!(
        out.get("filter").is_none(),
        "must not inject a filter param"
    );
    assert_eq!(out["limit"], 50, "caller args preserved");
}
