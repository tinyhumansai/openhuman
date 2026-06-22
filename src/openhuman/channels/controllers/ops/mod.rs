//! Channel controller business logic.

mod connect;
mod discord;
mod messaging;
mod telegram;
mod types;
mod yuanbao;

// Re-export public types.
pub use types::{
    ChannelConnectionResult, ChannelStatusEntry, ChannelTestResult, DiscordLinkCheckResult,
    DiscordLinkStartResult, TelegramLoginCheckResult, TelegramLoginStartResult,
};

// Re-export types needed by tests.
#[cfg(test)]
pub(crate) use crate::openhuman::channels::controllers::{ChannelAuthMode, ChannelDefinition};
#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use connect::channel_config_connected;
#[cfg(test)]
pub(crate) use connect::credential_provider;
#[cfg(test)]
pub(crate) use connect::merge_listener_health;
#[cfg(test)]
pub(crate) use connect::parse_allowed_users;

// Re-export public ops functions.
pub use connect::{
    channel_status, connect_channel, connected_channel_slugs, describe_channel, disconnect_channel,
    get_default_channel, list_channels, set_default_channel, test_channel,
};
pub use discord::{
    discord_check_permissions, discord_link_check, discord_link_start, discord_list_channels,
    discord_list_guilds,
};
pub use messaging::{
    channel_create_thread, channel_list_threads, channel_send_message, channel_send_reaction,
    channel_update_thread,
};
pub use telegram::{telegram_login_check, telegram_login_start};

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
