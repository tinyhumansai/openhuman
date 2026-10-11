use super::*;
use serde_json::json;

fn params() -> RelayInboundParams {
    serde_json::from_value(json!({
        "channel": "telegram",
        "chat_id": "-100123",
        "sender_id": "42",
        "sender_name": "Ada",
        "message_id": "m-1",
        "text": "hello",
    }))
    .expect("params")
}

#[test]
fn a_well_formed_message_is_accepted() {
    let p = params();
    p.validate().expect("valid");
    assert_eq!(p.client_id(), DEFAULT_RELAY_CLIENT_ID);
    assert_eq!(p.thread_title(), "telegram · Ada · -100123");
}

#[test]
fn the_thread_id_is_a_reserved_channel_id() {
    let p = params();
    let id = p.thread_id();
    assert_eq!(id, "channel:telegram/42/-100123");
    assert_eq!(
        id,
        tinychannels::channel::derive_inbound_thread_id(
            "telegram",
            Some("42"),
            Some("-100123"),
            None
        ),
        "the backend-relayed inbound path's derivation"
    );
    assert!(
        crate::profiles::surface::validate_user_thread_id(&id).is_err(),
        "a user cannot choose it"
    );
}

#[test]
fn distinct_chats_and_senders_get_distinct_threads() {
    let base = params();
    let mut other_chat = params();
    other_chat.chat_id = "-100124".into();
    let mut other_sender = params();
    other_sender.sender_id = "43".into();
    assert_ne!(base.thread_id(), other_chat.thread_id());
    assert_ne!(base.thread_id(), other_sender.thread_id());
}

#[test]
fn channel_names_are_lowercase_tokens() {
    for ok in ["telegram", "imessage", "discord", "x", "a_b-1"] {
        validate_channel(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for bad in [
        "",
        "Telegram",
        "1telegram",
        "tele gram",
        "telegram:bot",
        "telegram/1",
        "tg#1",
        "télégram",
        &"a".repeat(MAX_CHANNEL_LEN + 1),
    ] {
        assert!(validate_channel(bad).is_err(), "{bad:?}");
    }
    for reserved in RESERVED_CHANNELS {
        assert!(validate_channel(reserved).is_err(), "{reserved}");
    }
}

#[test]
fn ids_cannot_forge_another_chats_thread() {
    // `a/b` + `c` and `a` + `b/c` would derive the same thread id.
    for bad in ["a/b", "a#thread:1", "a\\b", "has space", "", "tab\t"] {
        assert!(validate_id("chat_id", bad).is_err(), "{bad:?}");
    }
    assert!(validate_id("chat_id", &"9".repeat(MAX_ID_LEN + 1)).is_err());
    for ok in [
        "-100123",
        "+15551234567",
        "ada@example.com",
        "iMessage;+;chat1",
    ] {
        validate_id("chat_id", ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
}

/// A change that breaks one field of a valid message.
type Mutation = Box<dyn Fn(&mut RelayInboundParams)>;

#[test]
fn every_field_is_checked() {
    let cases: Vec<(&str, Mutation)> = vec![
        ("channel", Box::new(|p| p.channel = "Web".into())),
        ("chat_id", Box::new(|p| p.chat_id = "a/b".into())),
        ("sender_id", Box::new(|p| p.sender_id = String::new())),
        ("message_id", Box::new(|p| p.message_id = "x y".into())),
        ("client_id", Box::new(|p| p.client_id = Some("c/1".into()))),
        (
            "sender_name",
            Box::new(|p| p.sender_name = Some("a".repeat(MAX_SENDER_NAME_CHARS + 1))),
        ),
        (
            "sender_name",
            Box::new(|p| p.sender_name = Some("a\nb".into())),
        ),
        ("text", Box::new(|p| p.text = "   ".into())),
        (
            "text",
            Box::new(|p| p.text = "x".repeat(MAX_TEXT_BYTES + 1)),
        ),
        (
            "attachments",
            Box::new(|p| p.attachments = Some(vec![json!({"url": "https://x"})])),
        ),
    ];
    for (field, mutate) in cases {
        let mut p = params();
        mutate(&mut p);
        let err = p.validate().expect_err(field);
        assert!(err.contains(field), "{field}: {err}");
    }
    let mut p = params();
    p.attachments = Some(vec![]);
    p.validate().expect("an empty attachment list is fine");
}

#[test]
fn the_channel_message_carries_the_relayed_facts() {
    let msg = params().to_channel_message();
    assert_eq!(msg.channel, "telegram");
    assert_eq!(msg.id, "m-1");
    assert_eq!(msg.sender, "42");
    assert_eq!(msg.reply_target, "-100123");
    assert_eq!(msg.content, "hello");
    assert_eq!(msg.sender_name.as_deref(), Some("Ada"));
    assert!(msg.thread_ts.is_none());
}

#[test]
fn a_constructed_message_round_trips_through_the_wire_shape() {
    let built = RelayInboundParams::new("telegram", "-100123", "42", "m-1", "hello");
    assert_eq!(built.client_id(), DEFAULT_RELAY_CLIENT_ID);
    assert!(built.validate().is_ok());
    let wire = serde_json::to_value(&built).expect("serialize");
    // Unset optional fields stay off the wire rather than arriving as `null`.
    assert!(wire.get("sender_name").is_none(), "{wire}");
    assert!(wire.get("attachments").is_none(), "{wire}");
    let back: RelayInboundParams = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, built);
    assert_eq!(back.thread_id(), params().thread_id());
}
