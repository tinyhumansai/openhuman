use super::channel_has_approval_surface;

// Sub-issue 2 of #3098: this gate is what decides whether the dispatch
// loop sets an `ApprovalChatContext` (→ gate fires for `Prompt`-class
// tools) versus the legacy bypass (→ tool calls silently allowed).
// Pin the matrix so silently broadening to a new channel can't
// accidentally TTL-deny every parked tool call there.

#[test]
fn telegram_has_approval_surface() {
    assert!(channel_has_approval_surface("telegram"));
}

#[test]
fn other_channels_do_not_yet_have_an_approval_surface() {
    for channel in ["discord", "slack", "imessage", "mattermost", "web", "irc"] {
        assert!(
            !channel_has_approval_surface(channel),
            "channel {channel:?} is not (yet) wired to a per-channel approval surface; \
             the dispatch loop must not scope an ApprovalChatContext for it or every \
             Prompt-class tool call will park with nobody to answer and TTL-deny"
        );
    }
}

#[test]
fn unknown_channel_does_not_have_approval_surface() {
    assert!(!channel_has_approval_surface(""));
    assert!(!channel_has_approval_surface("Telegram")); // case-sensitive on purpose
    assert!(!channel_has_approval_surface("telegram-bot"));
}
