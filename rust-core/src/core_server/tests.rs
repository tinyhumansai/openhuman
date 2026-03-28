use serde_json::json;

use crate::core_server::{
    call_method,
    types::CommandResponse,
    AccessibilityStatus, AutocompleteStatus,
};

#[tokio::test]
async fn accessibility_status_rpc_returns_valid_schema() {
    let raw = call_method("openhuman.accessibility_status", json!({}))
        .await
        .expect("status rpc should return");
    let payload: CommandResponse<AccessibilityStatus> =
        serde_json::from_value(raw).expect("status payload should decode");

    assert!(!payload.logs.is_empty());
    assert!(!payload.result.config.capture_policy.is_empty());
}

#[tokio::test]
async fn accessibility_start_session_requires_consent() {
    let err = call_method(
        "openhuman.accessibility_start_session",
        json!({
            "consent": false,
            "ttl_secs": 60
        }),
    )
    .await
    .expect_err("session start without consent should fail");

    assert!(err.contains("consent"));
}

#[tokio::test]
async fn accessibility_input_action_rejects_invalid_envelope() {
    let err = call_method(
        "openhuman.accessibility_input_action",
        json!({
            "x": 10,
            "y": 20
        }),
    )
    .await
    .expect_err("missing action should fail envelope validation");

    assert!(err.contains("invalid params"));
}

#[tokio::test]
async fn autocomplete_status_rpc_returns_valid_schema() {
    let raw = call_method("openhuman.autocomplete_status", json!({}))
        .await
        .expect("autocomplete status rpc should return");
    let payload: CommandResponse<AutocompleteStatus> =
        serde_json::from_value(raw).expect("autocomplete status payload should decode");

    assert!(!payload.logs.is_empty());
}
