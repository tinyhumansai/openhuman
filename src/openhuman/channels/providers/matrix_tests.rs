use super::*;

fn make_channel() -> MatrixChannel {
    MatrixChannel::new(
        "https://matrix.org".to_string(),
        "syt_test_token".to_string(),
        "!room:matrix.org".to_string(),
        vec!["@user:matrix.org".to_string()],
    )
}

#[test]
fn creates_with_correct_fields() {
    let ch = make_channel();
    assert_eq!(ch.homeserver, "https://matrix.org");
    assert_eq!(ch.access_token, "syt_test_token");
    assert_eq!(ch.room_id, "!room:matrix.org");
    assert_eq!(ch.allowed_users.len(), 1);
}

#[test]
fn strips_trailing_slash() {
    let ch = MatrixChannel::new(
        "https://matrix.org/".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org");
}

#[test]
fn no_trailing_slash_unchanged() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org");
}

#[test]
fn multiple_trailing_slashes_strip_all() {
    let ch = MatrixChannel::new(
        "https://matrix.org//".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org");
}

#[test]
fn trims_access_token() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "  syt_test_token  ".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.access_token, "syt_test_token");
}

#[test]
fn session_hints_are_normalized() {
    let ch = MatrixChannel::new_with_session_hint(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
        Some("  @bot:matrix.org ".to_string()),
        Some("  DEVICE123  ".to_string()),
    );

    assert_eq!(ch.session_owner_hint.as_deref(), Some("@bot:matrix.org"));
    assert_eq!(ch.session_device_id_hint.as_deref(), Some("DEVICE123"));
}

#[test]
fn empty_session_hints_are_ignored() {
    let ch = MatrixChannel::new_with_session_hint(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
        Some("   ".to_string()),
        Some(String::new()),
    );

    assert!(ch.session_owner_hint.is_none());
    assert!(ch.session_device_id_hint.is_none());
}

#[test]
fn encode_path_segment_encodes_room_refs() {
    assert_eq!(
        MatrixChannel::encode_path_segment("#ops:matrix.example.com"),
        "%23ops%3Amatrix.example.com"
    );
    assert_eq!(
        MatrixChannel::encode_path_segment("!room:matrix.example.com"),
        "%21room%3Amatrix.example.com"
    );
}

#[test]
fn supported_message_type_detection() {
    assert!(MatrixChannel::is_supported_message_type("m.text"));
    assert!(MatrixChannel::is_supported_message_type("m.notice"));
    assert!(!MatrixChannel::is_supported_message_type("m.image"));
    assert!(!MatrixChannel::is_supported_message_type("m.file"));
}

#[test]
fn body_presence_detection() {
    assert!(MatrixChannel::has_non_empty_body("hello"));
    assert!(MatrixChannel::has_non_empty_body("  hello  "));
    assert!(!MatrixChannel::has_non_empty_body(""));
    assert!(!MatrixChannel::has_non_empty_body("   \n\t  "));
}

#[test]
fn send_content_uses_markdown_formatting() {
    let content = RoomMessageEventContent::text_markdown("**hello**");
    let value = serde_json::to_value(content).unwrap();

    assert_eq!(value["msgtype"], "m.text");
    assert_eq!(value["body"], "**hello**");
    assert_eq!(value["format"], "org.matrix.custom.html");
    assert!(value["formatted_body"]
        .as_str()
        .unwrap_or_default()
        .contains("<strong>hello</strong>"));
}

#[test]
fn sync_filter_for_room_targets_requested_room() {
    let filter = MatrixChannel::sync_filter_for_room("!room:matrix.org", 0);
    let value: serde_json::Value = serde_json::from_str(&filter).unwrap();

    assert_eq!(value["room"]["rooms"][0], "!room:matrix.org");
    assert_eq!(value["room"]["timeline"]["limit"], 1);
}

#[test]
fn event_id_cache_deduplicates_and_evicts_old_entries() {
    let mut recent_order = std::collections::VecDeque::new();
    let mut recent_lookup = std::collections::HashSet::new();

    assert!(!MatrixChannel::cache_event_id(
        "$first:event",
        &mut recent_order,
        &mut recent_lookup
    ));
    assert!(MatrixChannel::cache_event_id(
        "$first:event",
        &mut recent_order,
        &mut recent_lookup
    ));

    for i in 0..2050 {
        let event_id = format!("$event-{i}:matrix");
        MatrixChannel::cache_event_id(&event_id, &mut recent_order, &mut recent_lookup);
    }

    assert!(!MatrixChannel::cache_event_id(
        "$first:event",
        &mut recent_order,
        &mut recent_lookup
    ));
}

#[test]
fn trims_room_id_and_allowed_users() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "  !room:matrix.org  ".to_string(),
        vec![
            "  @user:matrix.org  ".to_string(),
            "   ".to_string(),
            "@other:matrix.org".to_string(),
        ],
    );

    assert_eq!(ch.room_id, "!room:matrix.org");
    assert_eq!(ch.allowed_users.len(), 2);
    assert!(ch.allowed_users.contains(&"@user:matrix.org".to_string()));
    assert!(ch.allowed_users.contains(&"@other:matrix.org".to_string()));
}

#[test]
fn wildcard_allows_anyone() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec!["*".to_string()],
    );
    assert!(ch.is_user_allowed("@anyone:matrix.org"));
    assert!(ch.is_user_allowed("@hacker:evil.org"));
}

#[test]
fn specific_user_allowed() {
    let ch = make_channel();
    assert!(ch.is_user_allowed("@user:matrix.org"));
}

#[test]
fn unknown_user_denied() {
    let ch = make_channel();
    assert!(!ch.is_user_allowed("@stranger:matrix.org"));
    assert!(!ch.is_user_allowed("@evil:hacker.org"));
}

#[test]
fn user_case_insensitive() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec!["@User:Matrix.org".to_string()],
    );
    assert!(ch.is_user_allowed("@user:matrix.org"));
    assert!(ch.is_user_allowed("@USER:MATRIX.ORG"));
}

#[test]
fn empty_allowlist_denies_all() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert!(!ch.is_user_allowed("@anyone:matrix.org"));
}

#[test]
fn name_returns_matrix() {
    let ch = make_channel();
    assert_eq!(ch.name(), "matrix");
}

#[test]
fn sync_response_deserializes_empty() {
    let json = r#"{"next_batch":"s123","rooms":{"join":{}}}"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.next_batch, "s123");
    assert!(resp.rooms.join.is_empty());
}

#[test]
fn sync_response_deserializes_with_events() {
    let json = r#"{
        "next_batch": "s456",
        "rooms": {
            "join": {
                "!room:matrix.org": {
                    "timeline": {
                        "events": [
                            {
                                "type": "m.room.message",
                                "event_id": "$event:matrix.org",
                                "sender": "@user:matrix.org",
                                "content": {
                                    "msgtype": "m.text",
                                    "body": "Hello!"
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.next_batch, "s456");
    let room = resp.rooms.join.get("!room:matrix.org").unwrap();
    assert_eq!(room.timeline.events.len(), 1);
    assert_eq!(room.timeline.events[0].sender, "@user:matrix.org");
    assert_eq!(
        room.timeline.events[0].event_id.as_deref(),
        Some("$event:matrix.org")
    );
    assert_eq!(
        room.timeline.events[0].content.body.as_deref(),
        Some("Hello!")
    );
    assert_eq!(
        room.timeline.events[0].content.msgtype.as_deref(),
        Some("m.text")
    );
}

#[test]
fn sync_response_ignores_non_text_events() {
    let json = r#"{
        "next_batch": "s789",
        "rooms": {
            "join": {
                "!room:m": {
                    "timeline": {
                        "events": [
                            {
                                "type": "m.room.member",
                                "sender": "@user:m",
                                "content": {}
                            }
                        ]
                    }
                }
            }
        }
    }"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    let room = resp.rooms.join.get("!room:m").unwrap();
    assert_eq!(room.timeline.events[0].event_type, "m.room.member");
    assert!(room.timeline.events[0].content.body.is_none());
}

#[test]
fn whoami_response_deserializes() {
    let json = r#"{"user_id":"@bot:matrix.org"}"#;
    let resp: WhoAmIResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.user_id, "@bot:matrix.org");
}

#[test]
fn event_content_defaults() {
    let json = r#"{"type":"m.room.message","sender":"@u:m","content":{}}"#;
    let event: TimelineEvent = serde_json::from_str(json).unwrap();
    assert!(event.content.body.is_none());
    assert!(event.content.msgtype.is_none());
}

#[test]
fn event_content_supports_notice_msgtype() {
    let json = r#"{
        "type":"m.room.message",
        "sender":"@u:m",
        "event_id":"$notice:m",
        "content":{"msgtype":"m.notice","body":"Heads up"}
    }"#;
    let event: TimelineEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.content.msgtype.as_deref(), Some("m.notice"));
    assert_eq!(event.content.body.as_deref(), Some("Heads up"));
    assert_eq!(event.event_id.as_deref(), Some("$notice:m"));
}

#[tokio::test]
async fn invalid_room_reference_fails_fast() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "room_without_prefix".to_string(),
        vec![],
    );

    let err = ch.resolve_room_id().await.unwrap_err();
    assert!(err
        .to_string()
        .contains("must start with '!' (room ID) or '#' (room alias)"));
}

#[tokio::test]
async fn target_room_id_keeps_canonical_room_id_without_lookup() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "!canonical:matrix.org".to_string(),
        vec![],
    );

    let room_id = ch.target_room_id().await.unwrap();
    assert_eq!(room_id, "!canonical:matrix.org");
}

#[tokio::test]
async fn target_room_id_uses_cached_alias_resolution() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "#ops:matrix.org".to_string(),
        vec![],
    );

    *ch.resolved_room_id_cache.write().await = Some("!cached:matrix.org".to_string());
    let room_id = ch.target_room_id().await.unwrap();
    assert_eq!(room_id, "!cached:matrix.org");
}

#[test]
fn sync_response_missing_rooms_defaults() {
    let json = r#"{"next_batch":"s0"}"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert!(resp.rooms.join.is_empty());
}
