use super::*;
use serde_json::json;

#[test]
fn base64_encode_round_trips_json_error_bodies() {
    use base64::Engine;
    let encoded = base64_encode(r#"{"error":"nope"}"#);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .unwrap();
    assert_eq!(decoded, br#"{"error":"nope"}"#);
}

#[test]
fn channel_events_with_invalid_payload_are_dropped_without_panicking() {
    dispatch_socket_event("telegram:message", json!({"message":"missing channel"}));
    dispatch_socket_event(
        "telegram:message",
        json!({"channel":"tg:123", "message":"   "}),
    );
}

#[test]
fn unhandled_payload_is_logged_without_slicing_multibyte_data() {
    let mut payload = "a".repeat(490);
    payload.push('н');
    payload.push_str(&"b".repeat(20));
    let serialized = json!({"data":payload}).to_string();
    assert!(serialized.len() > 500);
    assert!(!serialized.is_char_boundary(500));
    dispatch_socket_event(
        "unrelated:event",
        serde_json::from_str(&serialized).unwrap(),
    );
}
