use super::*;

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
fn ack_reaction_is_deterministic_and_total_on_edge_inputs() {
    let a = select_acknowledgment_reaction("thanks");
    let b = select_acknowledgment_reaction("thanks");
    assert_eq!(a, b, "same input should always yield same reaction");
    // Empty input must not panic (`content.chars().next()` is None).
    assert!(!select_acknowledgment_reaction("").is_empty());
    // A single "?" falls into the question category.
    assert!(is_in(select_acknowledgment_reaction("?"), &["🤔", "✍️"]));
}

// ── build_channel_context_block (#928) ───────────────────────

fn cm(channel: &str, reply_target: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        channel: channel.into(),
        sender: "alice".into(),
        content: "hi".into(),
        id: "m1".into(),
        reply_target: reply_target.into(),
        thread_ts: None,
        timestamp: 0,
        sender_name: None,
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
    // Reminders return to this chat on their own: the hint says so and does
    // not ask the model to copy a delivery target.
    assert!(block.contains("delivered back to this chat automatically"));
    assert!(!block.contains("\"123456\""));
    assert!(!block.contains("announce"));
    assert!(!block.contains("cron_add"));
}

#[test]
fn channel_context_block_for_discord_and_slack_share_shape() {
    for ch in ["discord", "slack", "matrix"] {
        let block = build_channel_context_block(&cm(ch, "chan-42"));
        assert!(block.contains(ch), "missing channel name in `{ch}` block");
        assert!(block.contains("delivered back to this chat automatically"));
    }
}
