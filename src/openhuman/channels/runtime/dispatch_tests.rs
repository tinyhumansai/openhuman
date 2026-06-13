use super::*;

#[test]
fn contains_any_hits_at_least_one_word() {
    assert!(contains_any("hello world", &["world"]));
    assert!(contains_any("hello world", &["not there", "world"]));
}

#[test]
fn contains_any_returns_false_when_none_match() {
    assert!(!contains_any("hello world", &["nope"]));
    assert!(!contains_any("hello world", &[]));
}

#[test]
fn starts_with_any_detects_leading_prefix() {
    assert!(starts_with_any("hello world", &["hello"]));
    assert!(starts_with_any("hey you", &["yo", "hey"]));
}

#[test]
fn starts_with_any_returns_false_when_none_match() {
    assert!(!starts_with_any("bonjour", &["hello", "hey"]));
    assert!(!starts_with_any("x", &[]));
}

// ── select_acknowledgment_reaction ────────────────────────────

fn is_in(emoji: &str, options: &[&str]) -> bool {
    options.contains(&emoji)
}

#[test]
fn ack_reaction_gratitude_category() {
    for msg in ["thanks a lot", "Thank you", "THX friend", "I appreciate it"] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["❤️", "🙏"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_celebration_category() {
    for msg in ["amazing job", "this is awesome", "incredible!!"] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["🔥", "🎉"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_crypto_category() {
    for msg in ["BTC price today", "ETH pump", "gm on the defi timeline"] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["💯", "⚡"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_technical_category() {
    for msg in ["deploy the api", "debug this code", "rust question"] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["👨‍💻", "🤓"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_greeting_category() {
    for msg in ["hi there", "hello", "hey friend", "yo"] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["🤗", "😁"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_question_category() {
    for msg in [
        "what is this?",
        "how does it work",
        "can you help",
        "is this correct",
    ] {
        let r = select_acknowledgment_reaction(msg);
        assert!(is_in(r, &["🤔", "✍️"]), "`{msg}` → {r}");
    }
}

#[test]
fn ack_reaction_default_category() {
    let r = select_acknowledgment_reaction("the task is running");
    assert!(is_in(r, &["👀", "✍️"]));
}

#[test]
fn ack_reaction_is_deterministic() {
    let a = select_acknowledgment_reaction("thanks");
    let b = select_acknowledgment_reaction("thanks");
    assert_eq!(a, b, "same input should always yield same reaction");
}

#[test]
fn ack_reaction_handles_empty_input_without_panic() {
    // `content.chars().next()` is None on empty input — must not panic.
    let r = select_acknowledgment_reaction("");
    assert!(!r.is_empty());
}

#[test]
fn ack_reaction_handles_single_char() {
    let r = select_acknowledgment_reaction("?");
    // Single "?" falls into question category (contains '?').
    assert!(is_in(r, &["🤔", "✍️"]));
}

// ── build_channel_context_block (#928) ───────────────────────

fn cm(channel: &str, reply_target: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        channel: channel.into(),
        sender: "alice".into(),
        content: "hi".into(),
        id: "m1".into(),
        reply_target: reply_target.into(),
        thread_ts: None,
        timestamp: 0,
    }
}

#[test]
fn channel_context_block_omitted_for_web_and_cli() {
    assert!(build_channel_context_block(&cm("web", "1")).is_empty());
    assert!(build_channel_context_block(&cm("cli", "1")).is_empty());
    assert!(build_channel_context_block(&cm("WEB", "1")).is_empty());
    assert!(build_channel_context_block(&cm("", "1")).is_empty());
}

#[test]
fn channel_context_block_omitted_when_reply_target_missing() {
    assert!(build_channel_context_block(&cm("telegram", "")).is_empty());
    assert!(build_channel_context_block(&cm("telegram", "   ")).is_empty());
}

#[test]
fn channel_context_block_for_telegram_includes_routing_hint() {
    let block = build_channel_context_block(&cm("telegram", "123456"));
    assert!(block.contains("[Channel context]"));
    assert!(block.contains("\"telegram\""));
    assert!(block.contains("\"123456\""));
    // Hint must steer the model toward announce mode with the same channel/target.
    assert!(block.contains("announce"));
    assert!(block.contains("cron_add"));
}

#[test]
fn channel_context_block_for_discord_and_slack_share_shape() {
    for ch in ["discord", "slack", "matrix"] {
        let block = build_channel_context_block(&cm(ch, "chan-42"));
        assert!(block.contains(ch), "missing channel name in `{ch}` block");
        assert!(block.contains("chan-42"));
        assert!(block.contains("announce"));
    }
}
