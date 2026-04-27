use super::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, cancel_chat,
    event_session_id_for, inference_budget_exceeded_user_message,
    is_inference_budget_exceeded_error, json_output, key_for, normalize_model_override,
    optional_f64, optional_string, required_string, schemas, start_chat,
    subscribe_web_channel_events,
};
use crate::core::TypeSchema;

#[tokio::test]
async fn start_chat_validates_required_fields() {
    let err = start_chat("", "thread", "hello", None, None)
        .await
        .expect_err("client id should be required");
    assert!(err.contains("client_id is required"));

    let err = start_chat("client", "", "hello", None, None)
        .await
        .expect_err("thread id should be required");
    assert!(err.contains("thread_id is required"));

    let err = start_chat("client", "thread", "   ", None, None)
        .await
        .expect_err("message should be required");
    assert!(err.contains("message is required"));
}

#[tokio::test]
async fn cancel_chat_validates_required_fields() {
    let err = cancel_chat("", "thread")
        .await
        .expect_err("client id should be required");
    assert!(err.contains("client_id is required"));

    let err = cancel_chat("client", "")
        .await
        .expect_err("thread id should be required");
    assert!(err.contains("thread_id is required"));
}

#[test]
fn detects_backend_budget_exhaustion_error() {
    assert!(is_inference_budget_exceeded_error(
        "OpenHuman API error (402 Payment Required): Budget exceeded — add credits to continue."
    ));
    assert!(is_inference_budget_exceeded_error(
        "provider error: budget exceeded, please add credits"
    ));
    assert!(!is_inference_budget_exceeded_error(
        "OpenHuman API error (500): Internal server error"
    ));
}

#[test]
fn budget_exceeded_copy_mentions_top_up() {
    let message = inference_budget_exceeded_user_message();
    assert!(message.contains("top up"));
    assert!(message.contains("credits"));
}

// ── Schema catalog ────────────────────────────────────────────

#[test]
fn web_channel_catalog_has_chat_and_cancel() {
    let s = all_web_channel_controller_schemas();
    let c = all_web_channel_registered_controllers();
    assert_eq!(s.len(), c.len());
    assert_eq!(s.len(), 2);
    let fns: Vec<&str> = s.iter().map(|x| x.function).collect();
    assert!(fns.contains(&"web_chat"));
    assert!(fns.contains(&"web_cancel"));
}

#[test]
fn chat_schema_requires_client_thread_message() {
    let s = schemas("chat");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"client_id"));
    assert!(required.contains(&"thread_id"));
    assert!(required.contains(&"message"));
    // model_override and temperature must be optional.
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "model_override" && !f.required));
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "temperature" && !f.required));
}

#[test]
fn cancel_schema_requires_client_and_thread() {
    let s = schemas("cancel");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["client_id", "thread_id"]);
}

#[test]
fn unknown_schema_returns_unknown_fallback() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "channel");
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "error");
}

// ── Helpers ───────────────────────────────────────────────────

#[test]
fn key_for_combines_client_id_and_thread_id() {
    assert_eq!(key_for("c1", "t1"), "c1::t1");
    assert_eq!(key_for("", ""), "::");
}

#[test]
fn event_session_id_for_is_stable() {
    // Two calls with the same args must produce the same id.
    let a = event_session_id_for("c1", "t1");
    let b = event_session_id_for("c1", "t1");
    assert_eq!(a, b);
    // Different args → different id.
    let c = event_session_id_for("c2", "t1");
    assert_ne!(a, c);
}

#[test]
fn normalize_model_override_returns_none_for_empty_or_whitespace() {
    assert!(normalize_model_override(None).is_none());
    assert!(normalize_model_override(Some("".into())).is_none());
    assert!(normalize_model_override(Some("   ".into())).is_none());
}

#[test]
fn normalize_model_override_trims_value() {
    assert_eq!(
        normalize_model_override(Some("  gpt-4  ".into())),
        Some("gpt-4".to_string())
    );
}

// ── Broadcast events ──────────────────────────────────────────

#[test]
fn subscribe_web_channel_events_returns_receiver() {
    // Just confirm we can subscribe without panic.
    let _rx = subscribe_web_channel_events();
}

// ── Field builder helpers ─────────────────────────────────────

#[test]
fn required_string_marks_field_required() {
    let f = required_string("client_id", "c");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::String));
}

#[test]
fn optional_string_marks_field_optional() {
    let f = optional_string("model", "c");
    assert!(!f.required);
}

#[test]
fn optional_f64_marks_field_optional() {
    let f = optional_f64("temperature", "c");
    assert!(!f.required);
}

#[test]
fn json_output_is_required_json_field() {
    let f = json_output("ack", "c");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}
