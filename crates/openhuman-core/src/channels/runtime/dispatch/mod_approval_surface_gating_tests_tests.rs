use super::channel_has_approval_surface;

// Sub-issue 2 of #3098: this gate is what decides whether the dispatch
// loop sets an `ApprovalChatContext` (→ gate fires for `Prompt`-class
// tools) versus the legacy bypass (→ tool calls silently allowed).
// Pin the matrix: every chat channel is served by
// `ChannelApprovalSurfaceSubscriber`; channels with no conversational reply
// path must not be scoped, or every parked call would TTL-deny.

#[test]
fn chat_channels_have_an_approval_surface() {
    for channel in [
        "telegram",
        "discord",
        "slack",
        "imessage",
        "mattermost",
        "signal",
        "whatsapp",
        "irc",
    ] {
        assert!(channel_has_approval_surface(channel), "{channel}");
    }
}

#[test]
fn channels_without_a_reply_path_do_not_have_an_approval_surface() {
    for channel in ["email", "cli", "webhook", "web"] {
        assert!(!channel_has_approval_surface(channel), "{channel}");
    }
}

#[test]
fn unknown_channel_does_not_have_approval_surface() {
    assert!(!channel_has_approval_surface(""));
    assert!(!channel_has_approval_surface("Telegram")); // case-sensitive on purpose
    assert!(!channel_has_approval_surface("telegram-bot"));
}
