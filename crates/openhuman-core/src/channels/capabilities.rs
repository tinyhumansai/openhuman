//! Transitional per-provider capability table.
//!
//! TinyChannels removed its static `capabilities_for` table in favour of
//! per-instance `ChannelCapabilities` described by the module. Until the host
//! reads those descriptors, these four legacy flags keep their previous
//! values, keyed by provider id.

use tinychannels_bus::capabilities::provider_id;

/// The four pre-v2 interaction flags the host still branches on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HostChannelCapabilities {
    pub remote_control: bool,
    pub chat_approvals: bool,
    pub history_key_ignores_thread: bool,
}

const CHAT: HostChannelCapabilities = HostChannelCapabilities {
    remote_control: true,
    chat_approvals: true,
    history_key_ignores_thread: false,
};

/// Capabilities of the provider behind `channel` (`"tg:1"` resolves to telegram).
pub(crate) fn capabilities_for(channel: &str) -> HostChannelCapabilities {
    match provider_id(channel) {
        "telegram" => HostChannelCapabilities {
            history_key_ignores_thread: true,
            ..CHAT
        },
        "discord" | "slack" | "mattermost" | "imessage" | "signal" | "whatsapp"
        | "whatsapp_web" | "irc" | "dingtalk" | "qq" | "lark" | "linq" | "yuanbao" => CHAT,
        _ => HostChannelCapabilities::default(),
    }
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
