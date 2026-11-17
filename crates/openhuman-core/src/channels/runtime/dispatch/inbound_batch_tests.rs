use super::*;
use tinychannels_bus::{ChannelInboundEnvelope, ChannelMessage};

fn queued(seq: u64, id: &str, timestamp: u64, content: &str) -> QueuedInbound {
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: id.to_string(),
        sender: "alice".to_string(),
        reply_target: format!("reply-{id}"),
        content: content.to_string(),
        channel: "telegram".to_string(),
        timestamp,
        thread_ts: Some(format!("thread-{id}")),
        sender_name: None,
    };
    let envelope = ChannelInboundEnvelope {
        message_id: id.to_string(),
        text: content.to_string(),
        parts: vec![ContentPart::Text {
            text: content.to_string(),
            markdown_dialect: None,
        }],
        ..Default::default()
    };
    QueuedInbound {
        seq,
        inbound: AdmittedInbound {
            msg,
            envelope,
            history_key: "telegram_alice_reply".to_string(),
            canonical_session_key: "main:telegram".to_string(),
            session_key: "main:telegram".to_string(),
            tenant: Default::default(),
            native_target: None,
            typed_envelope_input: false,
            imported_history: Vec::new(),
            legacy_history: None,
        },
    }
}

#[test]
fn single_message_passes_through_unchanged() {
    let merged = merge_channel_batch(vec![queued(1, "m1", 5, "  hello  ")]).unwrap();
    assert_eq!(merged.msg.content, "  hello  ");
    assert_eq!(merged.msg.id, "m1");
    assert_eq!(merged.msg.thread_ts.as_deref(), Some("thread-m1"));
    assert_eq!(merged.history_key, "telegram_alice_reply");
    assert!(merge_channel_batch(Vec::new()).is_none());
}

#[test]
fn merge_orders_by_timestamp_then_receive_order_and_uses_newest_as_base() {
    let batch = vec![
        queued(3, "m3", 20, "third"),
        queued(2, "m2", 10, "second"),
        queued(1, "m1", 10, "first"),
    ];
    let merged = merge_channel_batch(batch).unwrap();
    assert_eq!(merged.msg.content, "first\nsecond\nthird");
    assert_eq!(merged.envelope.text, "first\nsecond\nthird");
    assert_eq!(merged.msg.id, "m3");
    assert_eq!(merged.msg.reply_target, "reply-m3");
    assert_eq!(merged.msg.thread_ts.as_deref(), Some("thread-m3"));
    assert_eq!(merged.msg.timestamp, 20);
}

#[test]
fn merged_typed_parts_keep_order_with_newline_separators() {
    let merged = merge_channel_batch(vec![
        queued(1, "m1", 1, "look"),
        queued(2, "m2", 2, "   "),
        queued(3, "m3", 3, "what is it?"),
    ])
    .unwrap();
    assert_eq!(merged.msg.content, "look\nwhat is it?");
    let texts: Vec<_> = merged
        .envelope
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["look", "\n", "what is it?"]);
}

#[test]
fn sender_name_is_the_last_one_present() {
    let mut a = queued(1, "m1", 1, "a");
    a.inbound.msg.sender_name = Some("Alice A".into());
    let mut b = queued(2, "m2", 2, "b");
    b.inbound.msg.sender_name = Some("Alice B".into());
    let c = queued(3, "m3", 3, "c");
    let merged = merge_channel_batch(vec![a, b, c]).unwrap();
    assert_eq!(merged.msg.sender_name.as_deref(), Some("Alice B"));
}

#[test]
fn a_typed_message_marks_the_merged_batch_typed() {
    let mut voice = queued(1, "m1", 1, "see attached");
    voice.inbound.typed_envelope_input = true;
    let merged = merge_channel_batch(vec![voice, queued(2, "m2", 2, "thanks")]).unwrap();
    assert!(merged.typed_envelope_input);
}

#[test]
fn item_traits_classify_media_commands_blanks_and_owned_state() {
    let image = queued(1, "m1", 1, "[IMAGE:x] caption");
    assert!(image.carries_media());
    assert!(image.mergeable());
    assert_eq!(image.chars(), "[IMAGE:x] caption".chars().count());
    assert!(queued(1, "m", 1, "[FILE:doc.pdf]").carries_media());
    assert!(!queued(1, "m", 1, "plain ✓").carries_media());
    assert_eq!(queued(1, "m", 1, "plain ✓").chars(), 7);
    assert!(!queued(1, "m", 1, "  /status").mergeable());
    assert!(!queued(1, "m", 1, " \n ").mergeable());

    let mut typed_media = queued(1, "m", 1, "see this");
    typed_media.inbound.envelope.parts.push(ContentPart::Image {
        media: Default::default(),
        caption: None,
        alt_text: None,
    });
    assert!(
        typed_media.carries_media(),
        "a typed image stretches the window"
    );

    let mut imported = queued(1, "m", 1, "hello");
    imported.inbound.imported_history =
        vec![tinyagents_session::transcript::TranscriptMessage::user(
            "old",
        )];
    assert!(
        !imported.mergeable(),
        "imported history belongs to one message"
    );
}

#[test]
fn a_captionless_typed_part_is_mergeable_but_a_blank_text_is_not() {
    let mut voice = queued(1, "m1", 1, "");
    voice.inbound.envelope.parts = vec![ContentPart::Image {
        media: Default::default(),
        caption: None,
        alt_text: None,
    }];
    assert!(voice.mergeable(), "an image with no caption still merges");
    assert!(!queued(1, "m2", 2, "   ").mergeable());
}
