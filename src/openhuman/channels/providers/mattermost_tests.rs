use super::*;
use serde_json::json;

// Helper: create a channel with mention_only=false (legacy behavior).
fn make_channel(allowed: Vec<String>, thread_replies: bool) -> MattermostChannel {
    MattermostChannel::new(
        "url".into(),
        "token".into(),
        None,
        allowed,
        thread_replies,
        false,
    )
}

// Helper: create a channel with mention_only=true.
fn make_mention_only_channel() -> MattermostChannel {
    MattermostChannel::new(
        "url".into(),
        "token".into(),
        None,
        vec!["*".into()],
        true,
        true,
    )
}

#[test]
fn mattermost_url_trimming() {
    let ch = MattermostChannel::new(
        "https://mm.example.com/".into(),
        "token".into(),
        None,
        vec![],
        false,
        false,
    );
    assert_eq!(ch.base_url, "https://mm.example.com");
}

#[test]
fn mattermost_allowlist_wildcard() {
    let ch = make_channel(vec!["*".into()], false);
    assert!(ch.is_user_allowed("any-id"));
}

#[test]
fn mattermost_parse_post_basic() {
    let ch = make_channel(vec!["*".into()], true);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "hello world",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789")
        .unwrap();
    assert_eq!(msg.sender, "user456");
    assert_eq!(msg.content, "hello world");
    assert_eq!(msg.reply_target, "chan789:post123"); // Default threaded reply
}

#[test]
fn mattermost_parse_post_thread_replies_enabled() {
    let ch = make_channel(vec!["*".into()], true);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "hello world",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789")
        .unwrap();
    assert_eq!(msg.reply_target, "chan789:post123"); // Threaded reply
}

#[test]
fn mattermost_parse_post_thread() {
    let ch = make_channel(vec!["*".into()], false);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "reply",
        "create_at": 1_600_000_000_000_i64,
        "root_id": "root789"
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789")
        .unwrap();
    assert_eq!(msg.reply_target, "chan789:root789"); // Stays in the thread
}

#[test]
fn mattermost_parse_post_ignore_self() {
    let ch = make_channel(vec!["*".into()], false);
    let post = json!({
        "id": "post123",
        "user_id": "bot123",
        "message": "my own message",
        "create_at": 1_600_000_000_000_i64
    });

    let msg =
        ch.parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789");
    assert!(msg.is_none());
}

#[test]
fn mattermost_parse_post_ignore_old() {
    let ch = make_channel(vec!["*".into()], false);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "old message",
        "create_at": 1_400_000_000_000_i64
    });

    let msg =
        ch.parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789");
    assert!(msg.is_none());
}

#[test]
fn mattermost_parse_post_no_thread_when_disabled() {
    let ch = make_channel(vec!["*".into()], false);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "hello world",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789")
        .unwrap();
    assert_eq!(msg.reply_target, "chan789"); // No thread suffix
}

#[test]
fn mattermost_existing_thread_always_threads() {
    // Even with thread_replies=false, replies to existing threads stay in the thread
    let ch = make_channel(vec!["*".into()], false);
    let post = json!({
        "id": "post123",
        "user_id": "user456",
        "message": "reply in thread",
        "create_at": 1_600_000_000_000_i64,
        "root_id": "root789"
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "botname", 1_500_000_000_000_i64, "chan789")
        .unwrap();
    assert_eq!(msg.reply_target, "chan789:root789"); // Stays in existing thread
}

// ── mention_only tests ────────────────────────────────────────

#[test]
fn mention_only_skips_message_without_mention() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "hello everyone",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch.parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1");
    assert!(msg.is_none());
}

#[test]
fn mention_only_accepts_message_with_at_mention() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "@mybot what is the weather?",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    assert_eq!(msg.content, "what is the weather?");
}

#[test]
fn mention_only_strips_mention_and_trims() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "  @mybot  run status  ",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    assert_eq!(msg.content, "run status");
}

#[test]
fn mention_only_rejects_empty_after_stripping() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "@mybot",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch.parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1");
    assert!(msg.is_none());
}

#[test]
fn mention_only_case_insensitive() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "@MyBot hello",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    assert_eq!(msg.content, "hello");
}

#[test]
fn mention_only_detects_metadata_mentions() {
    // Even without @username in text, metadata.mentions should trigger.
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "hey check this out",
        "create_at": 1_600_000_000_000_i64,
        "root_id": "",
        "metadata": {
            "mentions": ["bot123"]
        }
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    // Content is preserved as-is since no @username was in the text to strip.
    assert_eq!(msg.content, "hey check this out");
}

#[test]
fn mention_only_word_boundary_prevents_partial_match() {
    let ch = make_mention_only_channel();
    // "@mybotextended" should NOT match "@mybot" because it extends the username.
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "@mybotextended hello",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch.parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1");
    assert!(msg.is_none());
}

#[test]
fn mention_only_mention_in_middle_of_text() {
    let ch = make_mention_only_channel();
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "hey @mybot how are you?",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    assert_eq!(msg.content, "hey   how are you?");
}

#[test]
fn mention_only_disabled_passes_all_messages() {
    // With mention_only=false (default), messages pass through unfiltered.
    let ch = make_channel(vec!["*".into()], true);
    let post = json!({
        "id": "post1",
        "user_id": "user1",
        "message": "no mention here",
        "create_at": 1_600_000_000_000_i64,
        "root_id": ""
    });

    let msg = ch
        .parse_mattermost_post(&post, "bot123", "mybot", 1_500_000_000_000_i64, "chan1")
        .unwrap();
    assert_eq!(msg.content, "no mention here");
}

// ── contains_bot_mention_mm unit tests ────────────────────────

#[test]
fn contains_mention_text_at_end() {
    let post = json!({});
    assert!(contains_bot_mention_mm(
        "hello @mybot",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn contains_mention_text_at_start() {
    let post = json!({});
    assert!(contains_bot_mention_mm(
        "@mybot hello",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn contains_mention_text_alone() {
    let post = json!({});
    assert!(contains_bot_mention_mm("@mybot", "bot123", "mybot", &post));
}

#[test]
fn no_mention_different_username() {
    let post = json!({});
    assert!(!contains_bot_mention_mm(
        "@otherbot hello",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn no_mention_partial_username() {
    let post = json!({});
    // "mybot" is a prefix of "mybotx" — should NOT match
    assert!(!contains_bot_mention_mm(
        "@mybotx hello",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn mention_detects_later_valid_mention_after_partial_prefix() {
    let post = json!({});
    assert!(contains_bot_mention_mm(
        "@mybotx ignore this, but @mybot handle this",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn mention_followed_by_punctuation() {
    let post = json!({});
    // "@mybot," — comma is not alphanumeric/underscore/dash/dot, so it's a boundary
    assert!(contains_bot_mention_mm(
        "@mybot, hello",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn mention_via_metadata_only() {
    let post = json!({
        "metadata": { "mentions": ["bot123"] }
    });
    assert!(contains_bot_mention_mm(
        "no at mention",
        "bot123",
        "mybot",
        &post
    ));
}

#[test]
fn no_mention_empty_username_no_metadata() {
    let post = json!({});
    assert!(!contains_bot_mention_mm("hello world", "bot123", "", &post));
}

// ── normalize_mattermost_content unit tests ───────────────────

#[test]
fn normalize_strips_and_trims() {
    let post = json!({});
    let result = normalize_mattermost_content("  @mybot  do stuff  ", "bot123", "mybot", &post);
    assert_eq!(result.as_deref(), Some("do stuff"));
}

#[test]
fn normalize_returns_none_for_no_mention() {
    let post = json!({});
    let result = normalize_mattermost_content("hello world", "bot123", "mybot", &post);
    assert!(result.is_none());
}

#[test]
fn normalize_returns_none_when_only_mention() {
    let post = json!({});
    let result = normalize_mattermost_content("@mybot", "bot123", "mybot", &post);
    assert!(result.is_none());
}

#[test]
fn normalize_preserves_text_for_metadata_mention() {
    let post = json!({
        "metadata": { "mentions": ["bot123"] }
    });
    let result = normalize_mattermost_content("check this out", "bot123", "mybot", &post);
    assert_eq!(result.as_deref(), Some("check this out"));
}

#[test]
fn normalize_strips_multiple_mentions() {
    let post = json!({});
    let result =
        normalize_mattermost_content("@mybot hello @mybot world", "bot123", "mybot", &post);
    assert_eq!(result.as_deref(), Some("hello   world"));
}

#[test]
fn normalize_keeps_partial_username_mentions() {
    let post = json!({});
    let result =
        normalize_mattermost_content("@mybot hello @mybotx world", "bot123", "mybot", &post);
    assert_eq!(result.as_deref(), Some("hello @mybotx world"));
}
