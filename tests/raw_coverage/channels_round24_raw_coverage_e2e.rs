//! Round24 raw integration coverage for broad cold channel helpers.
//!
//! Parser/state seams only: no real network traffic.

use openhuman_core::channels::test_support as runtime_support;
use serde_json::json;
use tinychannels::delivery::progressive::{
    extract_message_id, latest_thinking_snippet, StreamingState,
};
use tinychannels::providers::email_channel::test_support as email_support;
use tinychannels::ChannelMessage;

fn channel_message(channel: &str, reply_target: &str, content: &str) -> ChannelMessage {
    ChannelMessage {
        id: "msg-1".to_string(),
        sender: "sender-1".to_string(),
        reply_target: reply_target.to_string(),
        content: content.to_string(),
        channel: channel.to_string(),
        timestamp: 1,
        thread_ts: None,
        sender_name: None,
    }
}

#[test]
fn bus_state_helpers_cover_message_ids_drafts_snippets_and_thread_keys() {
    assert_eq!(
        extract_message_id(&json!({"id": "abc"})).as_deref(),
        Some("abc")
    );
    assert_eq!(
        extract_message_id(&json!({"messageId": 42})).as_deref(),
        Some("42")
    );
    assert_eq!(
        extract_message_id(&json!({"data": {"messageId": 99_u64}})).as_deref(),
        Some("99")
    );
    assert!(extract_message_id(&json!({"ok": true})).is_none());

    assert_eq!(StreamingState::default().compose_draft(), "_working…_");
    assert_eq!(
        StreamingState {
            content: "answer with trailing space   ".into(),
            ..Default::default()
        }
        .compose_draft(),
        "answer with trailing space"
    );

    assert!(latest_thinking_snippet(&StreamingState {
        thinking_accumulator: "   ".into(),
        ..Default::default()
    })
    .is_none());
    let long_thinking = format!("prefix {}", "alpha ".repeat(80));
    let snippet = latest_thinking_snippet(&StreamingState {
        thinking_accumulator: long_thinking,
        ..Default::default()
    })
    .expect("snippet");
    assert!(snippet.len() <= 200);
    assert!(snippet.starts_with("alpha"));

    assert_eq!(
        tinychannels::channel::derive_inbound_thread_id(
            "slack:T1",
            Some("  U1 "),
            Some(" C1 "),
            Some(" 1700.1 "),
        ),
        "channel:slack:T1/U1/C1#thread:1700.1"
    );
    assert_eq!(
        tinychannels::channel::derive_inbound_thread_id(
            "telegram:123",
            Some("U1"),
            Some("123"),
            Some("message-specific"),
        ),
        "channel:telegram:123/U1/123"
    );
}

#[test]
fn dispatch_helpers_cover_channel_context_and_ack_categories() {
    let web = channel_message("web", "thread-1", "hello");
    assert!(runtime_support::build_channel_context_block_for_test(&web).is_empty());

    let telegram = channel_message("telegram", "chat-42", "remind me tomorrow");
    let context = runtime_support::build_channel_context_block_for_test(&telegram);
    assert!(context.contains("telegram"));
    // The job records its origin, so the model is told reminders return to
    // this chat on their own and is not handed a target to copy.
    assert!(context.contains("delivered back to this chat automatically"));
    assert!(!context.contains("chat-42"));
    assert!(!context.contains("cron_add"));

    let no_target = channel_message("slack", "", "hello");
    assert!(runtime_support::build_channel_context_block_for_test(&no_target).is_empty());

    let gratitude = tinychannels::runtime::select_acknowledgment_reaction("thank you");
    assert!(["❤️", "🙏"].contains(&gratitude));
    let finance = tinychannels::runtime::select_acknowledgment_reaction("btc market?");
    assert!(["💯", "⚡"].contains(&finance));
    let code = tinychannels::runtime::select_acknowledgment_reaction("debug this rust api");
    assert!(["👨‍💻", "🤓"].contains(&code));
    let greeting = tinychannels::runtime::select_acknowledgment_reaction("hello there");
    assert!(["🤗", "😁"].contains(&greeting));
    let question = tinychannels::runtime::select_acknowledgment_reaction("what happened?");
    assert!(["🤔", "✍️"].contains(&question));
}

#[test]
fn email_parser_edges_are_exercised_without_sockets() {
    let no_sender = b"Subject: Anonymous\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody";
    let parsed_mail = email_support::parse_email_fixture(no_sender).expect("mail parse");
    assert_eq!(parsed_mail.sender, "unknown");
    assert_eq!(parsed_mail.text, "body");

    let html_only =
        b"From: Html <html@example.test>\r\nContent-Type: text/html\r\n\r\n<p>A</p><p>B</p>";
    let parsed_html = email_support::parse_email_fixture(html_only).expect("html parse");
    assert_eq!(parsed_html.text, "A\nB\n");
}
