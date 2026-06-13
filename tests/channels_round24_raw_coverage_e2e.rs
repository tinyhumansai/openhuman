//! Round24 raw integration coverage for broad cold channel helpers.
//!
//! Parser/state seams only: no real network traffic.

use openhuman_core::openhuman::channels::bus::test_support as bus_support;
use openhuman_core::openhuman::channels::providers::email_channel::test_support as email_support;
use openhuman_core::openhuman::channels::providers::lark::test_support as lark_support;
use openhuman_core::openhuman::channels::providers::telegram::test_support as telegram_support;
use openhuman_core::openhuman::channels::test_support as runtime_support;
use openhuman_core::openhuman::channels::traits::ChannelMessage;
use serde_json::json;

fn channel_message(channel: &str, reply_target: &str, content: &str) -> ChannelMessage {
    ChannelMessage {
        id: "msg-1".to_string(),
        sender: "sender-1".to_string(),
        reply_target: reply_target.to_string(),
        content: content.to_string(),
        channel: channel.to_string(),
        timestamp: 1,
        thread_ts: None,
    }
}

#[test]
fn bus_state_helpers_cover_message_ids_drafts_snippets_and_thread_keys() {
    assert_eq!(
        bus_support::extract_message_id_for_test(&json!({"id": "abc"})).as_deref(),
        Some("abc")
    );
    assert_eq!(
        bus_support::extract_message_id_for_test(&json!({"messageId": 42})).as_deref(),
        Some("42")
    );
    assert_eq!(
        bus_support::extract_message_id_for_test(&json!({"data": {"messageId": 99_u64}}))
            .as_deref(),
        Some("99")
    );
    assert!(bus_support::extract_message_id_for_test(&json!({"ok": true})).is_none());

    assert_eq!(bus_support::compose_draft_for_test(""), "_working…_");
    assert_eq!(
        bus_support::compose_draft_for_test("answer with trailing space   "),
        "answer with trailing space"
    );

    assert!(bus_support::latest_thinking_snippet_for_test("   ").is_none());
    let long_thinking = format!("prefix {}", "alpha ".repeat(80));
    let snippet = bus_support::latest_thinking_snippet_for_test(&long_thinking).expect("snippet");
    assert!(snippet.len() <= 200);
    assert!(snippet.starts_with("alpha"));

    assert_eq!(
        bus_support::derive_inbound_thread_id_for_test(
            "slack:T1",
            Some("  U1 "),
            Some(" C1 "),
            Some(" 1700.1 "),
        ),
        "channel:slack:T1/U1/C1#thread:1700.1"
    );
    assert_eq!(
        bus_support::derive_inbound_thread_id_for_test(
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
    assert!(context.contains("chat-42"));
    assert!(context.contains("cron_add"));

    let no_target = channel_message("slack", "", "hello");
    assert!(runtime_support::build_channel_context_block_for_test(&no_target).is_empty());

    let gratitude = runtime_support::select_acknowledgment_reaction_for_test("thank you");
    assert!(["❤️", "🙏"].contains(&gratitude));
    let finance = runtime_support::select_acknowledgment_reaction_for_test("btc market?");
    assert!(["💯", "⚡"].contains(&finance));
    let code = runtime_support::select_acknowledgment_reaction_for_test("debug this rust api");
    assert!(["👨‍💻", "🤓"].contains(&code));
    let greeting = runtime_support::select_acknowledgment_reaction_for_test("hello there");
    assert!(["🤗", "😁"].contains(&greeting));
    let question = runtime_support::select_acknowledgment_reaction_for_test("what happened?");
    assert!(["🤔", "✍️"].contains(&question));
}

#[test]
fn lark_and_email_parser_edges_are_exercised_without_sockets() {
    let rich_post = json!({
        "en_us": {
            "content": [[
                {"tag": "a", "href": "https://example.test/fallback"},
                {"tag": "at", "user_id": "ou_123"},
                {"tag": "text", "text": " done"}
            ]]
        }
    })
    .to_string();
    let parsed = lark_support::parse_post_content_for_test(&rich_post).expect("post text");
    assert!(parsed.contains("https://example.test/fallback"));
    assert!(parsed.contains("@ou_123"));
    assert!(parsed.ends_with("done"));
    assert!(lark_support::parse_post_content_for_test("not json").is_none());
    assert_eq!(
        lark_support::strip_at_placeholders_for_test("before @_user_123 after @_user_x"),
        "before after @_user_x"
    );

    let no_sender = b"Subject: Anonymous\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody";
    let parsed_mail = email_support::parse_email_fixture(no_sender).expect("mail parse");
    assert_eq!(parsed_mail.sender, "unknown");
    assert_eq!(parsed_mail.text, "body");

    let html_only =
        b"From: Html <html@example.test>\r\nContent-Type: text/html\r\n\r\n<p>A</p><p>B</p>";
    let parsed_html = email_support::parse_email_fixture(html_only).expect("html parse");
    assert_eq!(parsed_html.text, "A\nB\n");
}

#[test]
fn telegram_reaction_marker_parser_covers_malformed_and_inline_forms() {
    assert_eq!(
        telegram_support::parse_reaction_marker_for_test("plain text"),
        ("plain text".to_string(), None)
    );
    assert_eq!(
        telegram_support::parse_reaction_marker_for_test("[REACTION:]"),
        (String::new(), None)
    );
    assert_eq!(
        telegram_support::parse_reaction_marker_for_test("[REACTION:ok"),
        ("[REACTION:ok".to_string(), None)
    );
    assert_eq!(
        telegram_support::parse_reaction_marker_for_test("  [REACTION:ok|123] reply body  "),
        ("reply body".to_string(), Some("ok|123".to_string()))
    );
}
