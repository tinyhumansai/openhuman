use super::*;

fn make_channel() -> SignalChannel {
    SignalChannel::new(
        "http://127.0.0.1:8686".to_string(),
        "+1234567890".to_string(),
        None,
        vec!["+1111111111".to_string()],
        false,
        false,
    )
}

fn make_channel_with_group(group_id: &str) -> SignalChannel {
    SignalChannel::new(
        "http://127.0.0.1:8686".to_string(),
        "+1234567890".to_string(),
        Some(group_id.to_string()),
        vec!["*".to_string()],
        true,
        true,
    )
}

fn make_envelope(source_number: Option<&str>, message: Option<&str>) -> Envelope {
    Envelope {
        source: source_number.map(String::from),
        source_number: source_number.map(String::from),
        data_message: message.map(|m| DataMessage {
            message: Some(m.to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    }
}

#[test]
fn creates_with_correct_fields() {
    let ch = make_channel();
    assert_eq!(ch.http_url, "http://127.0.0.1:8686");
    assert_eq!(ch.account, "+1234567890");
    assert!(ch.group_id.is_none());
    assert_eq!(ch.allowed_from.len(), 1);
    assert!(!ch.ignore_attachments);
    assert!(!ch.ignore_stories);
}

#[test]
fn strips_trailing_slash() {
    let ch = SignalChannel::new(
        "http://127.0.0.1:8686/".to_string(),
        "+1234567890".to_string(),
        None,
        vec![],
        false,
        false,
    );
    assert_eq!(ch.http_url, "http://127.0.0.1:8686");
}

#[test]
fn wildcard_allows_anyone() {
    let ch = make_channel_with_group("dm");
    assert!(ch.is_sender_allowed("+9999999999"));
}

#[test]
fn specific_sender_allowed() {
    let ch = make_channel();
    assert!(ch.is_sender_allowed("+1111111111"));
}

#[test]
fn unknown_sender_denied() {
    let ch = make_channel();
    assert!(!ch.is_sender_allowed("+9999999999"));
}

#[test]
fn empty_allowlist_denies_all() {
    let ch = SignalChannel::new(
        "http://127.0.0.1:8686".to_string(),
        "+1234567890".to_string(),
        None,
        vec![],
        false,
        false,
    );
    assert!(!ch.is_sender_allowed("+1111111111"));
}

#[test]
fn name_returns_signal() {
    let ch = make_channel();
    assert_eq!(ch.name(), "signal");
}

#[test]
fn matches_group_no_group_id_accepts_all() {
    let ch = make_channel();
    let dm = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: None,
        attachments: None,
    };
    assert!(ch.matches_group(&dm));

    let group = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("group123".to_string()),
        }),
        attachments: None,
    };
    assert!(ch.matches_group(&group));
}

#[test]
fn matches_group_filters_group() {
    let ch = make_channel_with_group("group123");
    let matching = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("group123".to_string()),
        }),
        attachments: None,
    };
    assert!(ch.matches_group(&matching));

    let non_matching = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("other_group".to_string()),
        }),
        attachments: None,
    };
    assert!(!ch.matches_group(&non_matching));
}

#[test]
fn matches_group_dm_keyword() {
    let ch = make_channel_with_group("dm");
    let dm = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: None,
        attachments: None,
    };
    assert!(ch.matches_group(&dm));

    let group = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("group123".to_string()),
        }),
        attachments: None,
    };
    assert!(!ch.matches_group(&group));
}

#[test]
fn reply_target_dm() {
    let ch = make_channel();
    let dm = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: None,
        attachments: None,
    };
    assert_eq!(ch.reply_target(&dm, "+1111111111"), "+1111111111");
}

#[test]
fn reply_target_group() {
    let ch = make_channel();
    let group = DataMessage {
        message: Some("hi".to_string()),
        timestamp: Some(1000),
        group_info: Some(GroupInfo {
            group_id: Some("group123".to_string()),
        }),
        attachments: None,
    };
    assert_eq!(ch.reply_target(&group, "+1111111111"), "group:group123");
}

#[test]
fn parse_recipient_target_e164_is_direct() {
    assert_eq!(
        SignalChannel::parse_recipient_target("+1234567890"),
        RecipientTarget::Direct("+1234567890".to_string())
    );
}

#[test]
fn parse_recipient_target_prefixed_group_is_group() {
    assert_eq!(
        SignalChannel::parse_recipient_target("group:abc123"),
        RecipientTarget::Group("abc123".to_string())
    );
}

#[test]
fn parse_recipient_target_uuid_is_direct() {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    assert_eq!(
        SignalChannel::parse_recipient_target(uuid),
        RecipientTarget::Direct(uuid.to_string())
    );
}

#[test]
fn parse_recipient_target_non_e164_plus_is_group() {
    assert_eq!(
        SignalChannel::parse_recipient_target("+abc123"),
        RecipientTarget::Group("+abc123".to_string())
    );
}

#[test]
fn is_uuid_valid() {
    assert!(SignalChannel::is_uuid(
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
    ));
    assert!(SignalChannel::is_uuid(
        "00000000-0000-0000-0000-000000000000"
    ));
}

#[test]
fn is_uuid_invalid() {
    assert!(!SignalChannel::is_uuid("+1234567890"));
    assert!(!SignalChannel::is_uuid("not-a-uuid"));
    assert!(!SignalChannel::is_uuid("group:abc123"));
    assert!(!SignalChannel::is_uuid(""));
}

#[test]
fn sender_prefers_source_number() {
    let env = Envelope {
        source: Some("uuid-123".to_string()),
        source_number: Some("+1111111111".to_string()),
        data_message: None,
        story_message: None,
        timestamp: Some(1000),
    };
    assert_eq!(SignalChannel::sender(&env), Some("+1111111111".to_string()));
}

#[test]
fn sender_falls_back_to_source() {
    let env = Envelope {
        source: Some("uuid-123".to_string()),
        source_number: None,
        data_message: None,
        story_message: None,
        timestamp: Some(1000),
    };
    assert_eq!(SignalChannel::sender(&env), Some("uuid-123".to_string()));
}

#[test]
fn process_envelope_uuid_sender_dm() {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let ch = SignalChannel::new(
        "http://127.0.0.1:8686".to_string(),
        "+1234567890".to_string(),
        None,
        vec!["*".to_string()],
        false,
        false,
    );
    let env = Envelope {
        source: Some(uuid.to_string()),
        source_number: None,
        data_message: Some(DataMessage {
            message: Some("Hello from privacy user".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let msg = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.sender, uuid);
    assert_eq!(msg.reply_target, uuid);
    assert_eq!(msg.content, "Hello from privacy user");

    // Verify reply routing: UUID sender in DM should route as Direct
    let target = SignalChannel::parse_recipient_target(&msg.reply_target);
    assert_eq!(target, RecipientTarget::Direct(uuid.to_string()));
}

#[test]
fn process_envelope_uuid_sender_in_group() {
    let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let ch = SignalChannel::new(
        "http://127.0.0.1:8686".to_string(),
        "+1234567890".to_string(),
        Some("testgroup".to_string()),
        vec!["*".to_string()],
        false,
        false,
    );
    let env = Envelope {
        source: Some(uuid.to_string()),
        source_number: None,
        data_message: Some(DataMessage {
            message: Some("Group msg from privacy user".to_string()),
            timestamp: Some(1_700_000_000_000),
            group_info: Some(GroupInfo {
                group_id: Some("testgroup".to_string()),
            }),
            attachments: None,
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    let msg = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.sender, uuid);
    assert_eq!(msg.reply_target, "group:testgroup");

    // Verify reply routing: group message should still route as Group
    let target = SignalChannel::parse_recipient_target(&msg.reply_target);
    assert_eq!(target, RecipientTarget::Group("testgroup".to_string()));
}

#[test]
fn sender_none_when_both_missing() {
    let env = Envelope {
        source: None,
        source_number: None,
        data_message: None,
        story_message: None,
        timestamp: None,
    };
    assert_eq!(SignalChannel::sender(&env), None);
}

#[test]
fn process_envelope_valid_dm() {
    let ch = make_channel();
    let env = make_envelope(Some("+1111111111"), Some("Hello!"));
    let msg = ch.process_envelope(&env).unwrap();
    assert_eq!(msg.content, "Hello!");
    assert_eq!(msg.sender, "+1111111111");
    assert_eq!(msg.channel, "signal");
}

#[test]
fn process_envelope_denied_sender() {
    let ch = make_channel();
    let env = make_envelope(Some("+9999999999"), Some("Hello!"));
    assert!(ch.process_envelope(&env).is_none());
}

#[test]
fn process_envelope_empty_message() {
    let ch = make_channel();
    let env = make_envelope(Some("+1111111111"), Some(""));
    assert!(ch.process_envelope(&env).is_none());
}

#[test]
fn process_envelope_no_data_message() {
    let ch = make_channel();
    let env = make_envelope(Some("+1111111111"), None);
    assert!(ch.process_envelope(&env).is_none());
}

#[test]
fn process_envelope_skips_stories() {
    let ch = make_channel_with_group("dm");
    let mut env = make_envelope(Some("+1111111111"), Some("story text"));
    env.story_message = Some(serde_json::json!({}));
    assert!(ch.process_envelope(&env).is_none());
}

#[test]
fn process_envelope_skips_attachment_only() {
    let ch = make_channel_with_group("dm");
    let env = Envelope {
        source: Some("+1111111111".to_string()),
        source_number: Some("+1111111111".to_string()),
        data_message: Some(DataMessage {
            message: None,
            timestamp: Some(1_700_000_000_000),
            group_info: None,
            attachments: Some(vec![serde_json::json!({"contentType": "image/png"})]),
        }),
        story_message: None,
        timestamp: Some(1_700_000_000_000),
    };
    assert!(ch.process_envelope(&env).is_none());
}

#[test]
fn sse_envelope_deserializes() {
    let json = r#"{
        "envelope": {
            "source": "+1111111111",
            "sourceNumber": "+1111111111",
            "timestamp": 1700000000000,
            "dataMessage": {
                "message": "Hello Signal!",
                "timestamp": 1700000000000
            }
        }
    }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let env = sse.envelope.unwrap();
    assert_eq!(env.source_number.as_deref(), Some("+1111111111"));
    let dm = env.data_message.unwrap();
    assert_eq!(dm.message.as_deref(), Some("Hello Signal!"));
}

#[test]
fn sse_envelope_deserializes_group() {
    let json = r#"{
        "envelope": {
            "sourceNumber": "+2222222222",
            "dataMessage": {
                "message": "Group msg",
                "groupInfo": {
                    "groupId": "abc123"
                }
            }
        }
    }"#;
    let sse: SseEnvelope = serde_json::from_str(json).unwrap();
    let env = sse.envelope.unwrap();
    let dm = env.data_message.unwrap();
    assert_eq!(
        dm.group_info.as_ref().unwrap().group_id.as_deref(),
        Some("abc123")
    );
}

#[test]
fn envelope_defaults() {
    let json = r#"{}"#;
    let env: Envelope = serde_json::from_str(json).unwrap();
    assert!(env.source.is_none());
    assert!(env.source_number.is_none());
    assert!(env.data_message.is_none());
    assert!(env.story_message.is_none());
    assert!(env.timestamp.is_none());
}
