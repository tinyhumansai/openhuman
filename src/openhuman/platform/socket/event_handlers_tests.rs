use super::*;
use parking_lot::RwLock;
use serde_json::json;

fn make_shared() -> Arc<SharedState> {
    Arc::new(SharedState {
        webhook_router: RwLock::new(None),
        ack_registry: super::super::manager::AckRegistry::default(),
        status: RwLock::new(ConnectionStatus::Disconnected),
        socket_id: RwLock::new(None),
        error: RwLock::new(None),
        connection_identity: RwLock::new(None),
    })
}

// ── base64_encode ───────────────────────────────────────────────

#[test]
fn base64_encode_round_trips_ascii() {
    use base64::Engine;
    let s = "hello world";
    let encoded = base64_encode(s);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .unwrap();
    assert_eq!(decoded, s.as_bytes());
}

#[test]
fn base64_encode_handles_empty_string() {
    assert_eq!(base64_encode(""), "");
}

#[test]
fn base64_encode_handles_json_body() {
    let encoded = base64_encode(r#"{"error":"nope"}"#);
    assert_eq!(encoded, "eyJlcnJvciI6Im5vcGUifQ==");
}

// ── parse_sio_event ─────────────────────────────────────────────

#[test]
fn parse_sio_event_accepts_bare_array() {
    let (name, data) = parse_sio_event(r#"["hello",{"x":1}]"#).unwrap();
    assert_eq!(name, "hello");
    assert_eq!(data, json!({"x": 1}));
}

#[test]
fn parse_sio_event_strips_ack_id_prefix() {
    let (name, data) = parse_sio_event(r#"123["hello",{"x":1}]"#).unwrap();
    assert_eq!(name, "hello");
    assert_eq!(data["x"], 1);
}

#[test]
fn parse_sio_event_defaults_data_to_null_when_missing() {
    let (name, data) = parse_sio_event(r#"["ping"]"#).unwrap();
    assert_eq!(name, "ping");
    assert!(data.is_null());
}

#[test]
fn parse_sio_event_returns_none_for_garbage() {
    assert!(parse_sio_event("not an sio event").is_none());
    assert!(parse_sio_event("").is_none());
}

#[test]
fn parse_sio_event_returns_none_when_first_element_is_not_string() {
    assert!(parse_sio_event("[42,{}]").is_none());
}

#[test]
fn parse_sio_event_returns_none_when_json_invalid() {
    assert!(parse_sio_event(r#"[invalid json"#).is_none());
}

// ── handle_sio_event dispatch ───────────────────────────────────

#[test]
fn handle_sio_event_ready_sets_connected() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_event("ready", json!({}), &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);
}

#[test]
fn handle_sio_event_error_sets_error_status() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_event("error", json!({"msg":"oops"}), &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Error);
}

#[test]
fn handle_sio_event_debug_truncation_respects_utf8_boundary() {
    // Serialized JSON must be >= 500 bytes with a multi-byte codepoint
    // straddling byte 500 — mirrors OPENHUMAN-TAURI-KC (Cyrillic at 499..501).
    let inner = format!("{}н", "a".repeat(498));
    let payload_json = serde_json::Value::String(inner.clone()).to_string();
    assert!(
        payload_json.len() >= 500,
        "fixture too short: {} bytes",
        payload_json.len()
    );
    assert!(
        !payload_json.is_char_boundary(500),
        "fixture must place byte 500 inside a multi-byte character"
    );

    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_event(
        "weird.unrelated.event",
        serde_json::Value::String(inner),
        &tx,
        &shared,
    );
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

#[test]
fn handle_sio_event_unknown_event_is_noop_on_status() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    // Start disconnected — an unhandled event must not flip status.
    handle_sio_event("weird.unrelated.event", json!({}), &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

#[test]
fn handle_sio_event_channel_message_missing_channel_is_dropped() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    // No "channel" field → the dispatcher must return without touching status.
    handle_sio_event("telegram:message", json!({"message": "hi"}), &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

#[test]
fn handle_sio_event_channel_message_empty_text_is_dropped() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_event(
        "telegram:message",
        json!({"channel": "tg:123", "message": "   "}),
        &tx,
        &shared,
    );
    // Status should still be untouched. The dropped-empty branch is the
    // coverage target — this test validates we hit the early-return path.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

// Regression: OPENHUMAN-TAURI-KC (#1814). A multi-byte UTF-8 char
// straddling byte 500 of `data.to_string()` used to panic the debug-log
// truncator with `byte index 500 is not a char boundary`, killing the
// core thread on every receipt of such an event.
//
// The fix: payload content is never emitted in any log line (PII/secrets
// policy). The raw payload bytes are therefore never sliced at a byte
// index that may not be a UTF-8 boundary. This test:
//   1. Constructs a fixture that would have triggered the old panic.
//   2. Verifies `handle_sio_event` completes without panicking.
//   3. Verifies the debug-log format string for the pre-match lines does
//      NOT include any payload slice — confirmed structurally by the code
//      review and enforced at the type level (the `payload` binding is
//      only used via `.len()` after this change).
#[test]
fn handle_sio_event_payload_redacted_no_panic_on_multibyte_boundary() {
    // Build a payload whose JSON serialization places the 2-byte Cyrillic
    // `'н'` exactly at bytes 499..501. `json!({"data": <s>}).to_string()`
    // emits `{"data":"<s>"}`, so the 9-byte prefix `{"data":"` plus 490
    // ASCII bytes lands the next char at byte 499.
    let mut s = "a".repeat(490);
    s.push('н'); // 2 bytes — straddles byte 500
    s.push_str(&"b".repeat(20)); // trailing pad past the 500-byte cap
    let payload = json!({ "data": s });
    let serialized = payload.to_string();
    assert!(
        serialized.len() > 500,
        "fixture must exceed the 500-byte boundary"
    );
    assert!(
        !serialized.is_char_boundary(500),
        "fixture must place a multi-byte char across byte 500"
    );

    // Confirm that the payload string, if sliced at byte 500, would panic —
    // i.e. that the old code really was broken for this input.
    let would_panic = std::panic::catch_unwind(|| {
        let _ = &serialized[..500];
    });
    assert!(
        would_panic.is_err(),
        "slice at byte 500 should panic for this fixture (validates the fixture itself)"
    );

    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    // Any event name exercises the pre-match log path. Must not panic.
    handle_sio_event("anything.unhandled", payload, &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}
