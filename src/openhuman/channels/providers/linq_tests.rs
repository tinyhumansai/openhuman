use super::*;

fn make_channel() -> LinqChannel {
    LinqChannel::new(
        "test-token".into(),
        "+15551234567".into(),
        vec!["+1234567890".into()],
    )
}

#[test]
fn linq_channel_name() {
    let ch = make_channel();
    assert_eq!(ch.name(), "linq");
}

#[test]
fn linq_sender_allowed_exact() {
    let ch = make_channel();
    assert!(ch.is_sender_allowed("+1234567890"));
    assert!(!ch.is_sender_allowed("+9876543210"));
}

#[test]
fn linq_sender_allowed_wildcard() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    assert!(ch.is_sender_allowed("+1234567890"));
    assert!(ch.is_sender_allowed("+9999999999"));
}

#[test]
fn linq_sender_allowed_empty() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec![]);
    assert!(!ch.is_sender_allowed("+1234567890"));
}

#[test]
fn linq_parse_valid_text_message() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "api_version": "v3",
        "event_type": "message.received",
        "event_id": "evt-123",
        "created_at": "2025-01-15T12:00:00Z",
        "trace_id": "trace-456",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "recipient_phone": "+15551234567",
            "is_from_me": false,
            "service": "iMessage",
            "message": {
                "id": "msg-abc",
                "parts": [{
                    "type": "text",
                    "value": "Hello OpenHuman!"
                }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "+1234567890");
    assert_eq!(msgs[0].content, "Hello OpenHuman!");
    assert_eq!(msgs[0].channel, "linq");
    assert_eq!(msgs[0].reply_target, "chat-789");
}

#[test]
fn linq_parse_skip_is_from_me() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": true,
            "message": {
                "id": "msg-abc",
                "parts": [{ "type": "text", "value": "My own message" }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "is_from_me messages should be skipped");
}

#[test]
fn linq_parse_skip_non_message_event() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "event_type": "message.delivered",
        "data": {
            "chat_id": "chat-789",
            "message_id": "msg-abc"
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Non-message events should be skipped");
}

#[test]
fn linq_parse_unauthorized_sender() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+9999999999",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{ "type": "text", "value": "Spam" }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Unauthorized senders should be filtered");
}

#[test]
fn linq_parse_empty_payload() {
    let ch = make_channel();
    let payload = serde_json::json!({});
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn linq_parse_media_only_translated_to_image_marker() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{
                    "type": "media",
                    "url": "https://example.com/image.jpg",
                    "mime_type": "image/jpeg"
                }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "[IMAGE:https://example.com/image.jpg]");
}

#[test]
fn linq_parse_media_non_image_still_skipped() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{
                    "type": "media",
                    "url": "https://example.com/sound.mp3",
                    "mime_type": "audio/mpeg"
                }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Non-image media should still be skipped");
}

#[test]
fn linq_parse_multiple_text_parts() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [
                    { "type": "text", "value": "First part" },
                    { "type": "text", "value": "Second part" }
                ]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "First part\nSecond part");
}

#[test]
fn linq_signature_verification_valid() {
    let secret = "test_webhook_secret";
    let body = r#"{"event_type":"message.received"}"#;
    let now = chrono::Utc::now().timestamp().to_string();

    // Compute expected signature
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let message = format!("{now}.{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    assert!(verify_linq_signature(secret, body, &now, &signature));
}

#[test]
fn linq_signature_verification_invalid() {
    let secret = "test_webhook_secret";
    let body = r#"{"event_type":"message.received"}"#;
    let now = chrono::Utc::now().timestamp().to_string();

    assert!(!verify_linq_signature(
        secret,
        body,
        &now,
        "deadbeefdeadbeefdeadbeef"
    ));
}

#[test]
fn linq_signature_verification_stale_timestamp() {
    let secret = "test_webhook_secret";
    let body = r#"{"event_type":"message.received"}"#;
    // 10 minutes ago — stale
    let stale_ts = (chrono::Utc::now().timestamp() - 600).to_string();

    // Even with correct signature, stale timestamp should fail
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let message = format!("{stale_ts}.{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    assert!(
        !verify_linq_signature(secret, body, &stale_ts, &signature),
        "Stale timestamps (>300s) should be rejected"
    );
}

#[test]
fn linq_signature_verification_accepts_sha256_prefix() {
    let secret = "test_webhook_secret";
    let body = r#"{"event_type":"message.received"}"#;
    let now = chrono::Utc::now().timestamp().to_string();

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let message = format!("{now}.{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    assert!(verify_linq_signature(secret, body, &now, &signature));
}

#[test]
fn linq_signature_verification_accepts_uppercase_hex() {
    let secret = "test_webhook_secret";
    let body = r#"{"event_type":"message.received"}"#;
    let now = chrono::Utc::now().timestamp().to_string();

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let message = format!("{now}.{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes()).to_ascii_uppercase();

    assert!(verify_linq_signature(secret, body, &now, &signature));
}

#[test]
fn linq_parse_normalizes_phone_with_plus() {
    let ch = LinqChannel::new(
        "tok".into(),
        "+15551234567".into(),
        vec!["+1234567890".into()],
    );
    // API sends without +, normalize to +
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{ "type": "text", "value": "Hi" }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "+1234567890");
}

#[test]
fn linq_parse_missing_data() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "event_type": "message.received"
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn linq_parse_missing_message_parts() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc"
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn linq_parse_empty_text_value() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-789",
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{ "type": "text", "value": "" }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Empty text should be skipped");
}

#[test]
fn linq_parse_fallback_reply_target_when_no_chat_id() {
    let ch = LinqChannel::new("tok".into(), "+15551234567".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "event_type": "message.received",
        "data": {
            "from": "+1234567890",
            "is_from_me": false,
            "message": {
                "id": "msg-abc",
                "parts": [{ "type": "text", "value": "Hi" }]
            }
        }
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    // Falls back to sender phone number when no chat_id
    assert_eq!(msgs[0].reply_target, "+1234567890");
}

#[test]
fn linq_phone_number_accessor() {
    let ch = make_channel();
    assert_eq!(ch.phone_number(), "+15551234567");
}
