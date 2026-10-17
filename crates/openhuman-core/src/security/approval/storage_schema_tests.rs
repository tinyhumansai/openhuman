use super::*;
use serde_json::json;
use tinystoragedrivers::Meta;

fn source(id: &str, body: Value) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        body,
        meta: Meta::unknown(),
    }
}

#[test]
fn a_pending_request_keeps_its_fields_and_gains_millisecond_times() {
    let out = requests_v2(vec![source(
        "req-1",
        json!({
            "tool_name": "shell",
            "action_summary": "run ls",
            "args_redacted": "{}",
            "session_id": "s",
            "created_at": "1970-01-01T00:00:01.000000000Z",
            "expires_at": "1970-01-01T00:00:03.000000000Z",
            "expires_ts": 3_000_000_000_i64,
            "pending": true,
            "tool_call_id": "c1",
            "agent_id": "a1",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, APPROVALS);
    assert_eq!(out[0].id, "req-1");
    assert_eq!(
        out[0].body,
        json!({
            "tool_name": "shell",
            "action_summary": "run ls",
            "args_redacted": "{}",
            "session_id": "s",
            "tool_call_id": "c1",
            "agent_id": "a1",
            "created_ms": 1000,
            "deadline_ms": 3000,
            "pending": true,
        })
    );
}

#[test]
fn a_decided_request_carries_its_audit_trail() {
    let out = requests_v2(vec![source(
        "req-2",
        json!({
            "tool_name": "shell", "action_summary": "x", "args_redacted": "{}",
            "session_id": "s", "created_at": "1970-01-01T00:00:01Z",
            "pending": false, "decided_at": "1970-01-01T00:00:02Z", "decision": "deny",
            "executed_at": "1970-01-01T00:00:04Z", "execution_outcome": "ok",
        }),
    )])
    .unwrap();
    let body = &out[0].body;
    assert_eq!(body["decided_ms"], 2000);
    assert_eq!(body["executed_ms"], 4000);
    assert_eq!(body["decision"], "deny");
    assert!(body.get("deadline_ms").is_none());
    assert!(body.get("expires_ts").is_none());
}

#[test]
fn a_flow_grant_is_rekeyed_by_its_two_parts() {
    let out = flow_trust_v2(vec![source(
        "4:flow/tool",
        json!({ "flow_id": "flow", "tool_name": "tool/x", "created_at": "1970-01-01T00:00:01Z" }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, FLOW_TRUST);
    assert_eq!(out[0].id, trust_id("flow", "tool/x"));
    assert_eq!(out[0].id, "flow/tool%2Fx");
    assert_eq!(
        out[0].body,
        json!({ "flow_id": "flow", "tool_name": "tool/x", "created_ms": 1000 })
    );
}

#[test]
fn a_malformed_grant_is_dropped() {
    let out = flow_trust_v2(vec![source("x", json!({ "flow_id": "f" }))]).unwrap();
    assert!(out.is_empty());
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
