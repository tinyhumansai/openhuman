use super::*;
use crate::openhuman::skills::webhooks::WebhookRequest;
use base64::Engine;
use std::collections::HashMap;

// ── Local helpers ─────────────────────────────────────────────

#[test]
fn base64_encode_matches_standard_engine_output() {
    assert_eq!(base64_encode("hello"), "aGVsbG8=");
    assert_eq!(base64_encode(""), "");
}

#[test]
fn error_body_is_base64_of_json_envelope() {
    let encoded = error_body("boom");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .expect("valid base64");
    let json: serde_json::Value = serde_json::from_slice(&decoded).expect("valid json");
    assert_eq!(json["error"].as_str(), Some("boom"));
}

// ── Constructor + EventHandler metadata ───────────────────────

#[test]
fn default_equals_new_and_is_zero_sized() {
    // Both constructors produce the same unit-variant struct.
    let _a = WebhookRequestSubscriber::default();
    let _b = WebhookRequestSubscriber::new();
    // Zero-sized type — just asserting both compile and construct.
    assert_eq!(std::mem::size_of::<WebhookRequestSubscriber>(), 0);
}

#[test]
fn event_handler_name_is_namespaced() {
    let s = WebhookRequestSubscriber::new();
    assert_eq!(s.name(), "webhook::request_handler");
}

#[test]
fn event_handler_domain_filter_is_webhook() {
    let s = WebhookRequestSubscriber::new();
    assert_eq!(s.domains(), Some(&["webhook"][..]));
}

// ── handle() behaviour ────────────────────────────────────────

#[tokio::test]
async fn handle_returns_early_on_non_webhook_event() {
    // A domain event for a different module must be ignored —
    // `handle()` checks the variant and returns without touching
    // the socket manager or publishing anything.
    let subscriber = WebhookRequestSubscriber::new();
    let event = DomainEvent::AgentTurnStarted {
        session_id: "s1".into(),
        channel: "web".into(),
    };
    // Must not panic, must not block — even without any singletons
    // initialised in the test process.
    subscriber.handle(&event).await;
}

#[tokio::test]
async fn handle_processes_incoming_webhook_without_socket_manager() {
    // When the socket-manager singleton isn't initialised, the router
    // lookup returns None (no registration), so the handler takes the
    // "no tunnel registration → 404" path and then logs "no socket
    // manager available" before returning cleanly.
    let subscriber = WebhookRequestSubscriber::new();
    let request = WebhookRequest {
        correlation_id: "wh_test_1".into(),
        tunnel_id: "tid-1".into(),
        tunnel_uuid: "uuid-unregistered".into(),
        tunnel_name: "my-hook".into(),
        method: "POST".into(),
        path: "/hook".into(),
        headers: HashMap::new(),
        query: HashMap::new(),
        body: String::new(),
    };
    let event = DomainEvent::WebhookIncomingRequest {
        request,
        raw_data: serde_json::json!({}),
    };
    // Must not panic — even without any singletons initialised.
    subscriber.handle(&event).await;
}

// ── decode_webhook_body ───────────────────────────────────────

#[test]
fn decode_webhook_body_empty_returns_empty_object() {
    let v = decode_webhook_body("").unwrap();
    assert!(v.as_object().map(|o| o.is_empty()).unwrap_or(false));
}

#[test]
fn decode_webhook_body_parses_valid_json() {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(r#"{"key":"value"}"#.as_bytes());
    let v = decode_webhook_body(&encoded).unwrap();
    assert_eq!(v["key"].as_str(), Some("value"));
}

#[test]
fn decode_webhook_body_wraps_non_json_in_raw_field() {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode("plain text".as_bytes());
    let v = decode_webhook_body(&encoded).unwrap();
    assert_eq!(v["raw"].as_str(), Some("plain text"));
}

#[test]
fn decode_webhook_body_rejects_invalid_base64() {
    let err = decode_webhook_body("not-valid-base64!!!").unwrap_err();
    assert!(err.contains("invalid base64"));
}

// ── build_agent_response ──────────────────────────────────────

#[test]
fn build_agent_response_sets_status_and_body() {
    let resp = build_agent_response("corr-1", 200, "Triage decision: drop");
    assert_eq!(resp.correlation_id, "corr-1");
    assert_eq!(resp.status_code, 200);
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    // Body must be base64-encoded JSON with a "result" key.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(resp.body.as_bytes())
        .expect("valid base64");
    let v: serde_json::Value = serde_json::from_slice(&decoded).expect("valid json");
    assert_eq!(v["result"].as_str(), Some("Triage decision: drop"));
}
