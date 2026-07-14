//! Phase 1 client effect-executor coverage: parsing the hosted brain's
//! `orch:effect:send_dm` frame, the ack frame it returns, the at-least-once
//! callId dedupe, and the device-tool manifest. Lives in an integration crate
//! (links the compiled lib) because the root cfg(test) build is blocked by
//! unrelated stale test modules at this checkout.

use openhuman_core::openhuman::orchestration::effect_executor::{
    device_tool_manifest, dispatch_device_tool, effect_result_frame, handle_tool_call,
    is_duplicate_call, parse_send_dm, parse_tool_call, tool_result_frame,
};
use serde_json::json;

#[test]
fn parses_a_well_formed_send_dm_frame() {
    let frame = json!({
        "cycleId": "cyc:agent-alice:sess-1:3",
        "callId": "cyc:agent-alice:sess-1:3:send_dm:0",
        "counterpartAgentId": "agent-alice",
        "sessionId": "sess-1",
        "body": "on it"
    });
    let effect = parse_send_dm(&frame).expect("parse");
    assert_eq!(effect.call_id, "cyc:agent-alice:sess-1:3:send_dm:0");
    assert_eq!(effect.counterpart_agent_id, "agent-alice");
    assert_eq!(effect.session_id, "sess-1");
    assert_eq!(effect.body, "on it");
}

#[test]
fn rejects_a_frame_missing_required_fields() {
    let frame = json!({ "cycleId": "c", "body": "hi" }); // no callId / counterpartAgentId
    assert!(parse_send_dm(&frame).is_err());
}

#[test]
fn ack_frame_shapes_ok_and_error_cases() {
    assert_eq!(
        effect_result_frame("call-1", true, None),
        json!({ "callId": "call-1", "ok": true, "error": null })
    );
    assert_eq!(
        effect_result_frame("call-2", false, Some("device offline")),
        json!({ "callId": "call-2", "ok": false, "error": "device offline" })
    );
}

#[test]
fn dedupe_reports_first_call_new_and_repeat_duplicate() {
    // Unique id so the process-global dedupe set can't collide with other tests.
    let id = "dedupe-test-unique-call-id-abc123";
    assert!(!is_duplicate_call(id), "first sighting is not a duplicate");
    assert!(is_duplicate_call(id), "second sighting is a duplicate");
}

#[test]
fn manifest_declares_a_queryable_device_tool() {
    let manifest = device_tool_manifest();
    let tools = manifest["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|t| t["name"] == "device_status"));
}

#[test]
fn parses_a_tool_call_frame() {
    let frame = json!({
        "cycleId": "c",
        "callId": "c:tool_call:0",
        "name": "device_status",
        "args": {}
    });
    let parsed = parse_tool_call(&frame).expect("parse");
    assert_eq!(parsed.call_id, "c:tool_call:0");
    assert_eq!(parsed.name, "device_status");
}

#[tokio::test]
async fn dispatches_device_status_and_rejects_unknown_tools() {
    let status = dispatch_device_tool("device_status", &json!({}), "master:session:cycle")
        .await
        .expect("ok");
    assert!(status["version"].is_string());
    assert!(status["platform"].is_string());

    assert!(
        dispatch_device_tool("rm_rf", &json!({}), "master:session:cycle")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn handle_tool_call_builds_result_frame() {
    let frame = json!({
        "cycleId": "master:session:cycle",
        "callId": "c:tool_call:0",
        "name": "device_status",
        "args": {}
    });
    let (call_id, result) = handle_tool_call(&frame).await.expect("handled");
    assert_eq!(call_id, "c:tool_call:0");
    assert_eq!(result["ok"], json!(true));
    assert!(result["result"]["platform"].is_string());
}

#[test]
fn tool_result_frame_shapes_error_case() {
    assert_eq!(
        tool_result_frame("c1", false, json!(null), Some("boom")),
        json!({ "callId": "c1", "ok": false, "result": null, "error": "boom" })
    );
}
