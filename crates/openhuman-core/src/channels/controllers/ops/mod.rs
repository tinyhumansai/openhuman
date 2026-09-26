//! Channel controller business logic.

mod connect;
mod discord;
mod messaging;
mod types;
mod yuanbao;

// Re-export types needed by tests.
#[cfg(test)]
pub(crate) use crate::channels::controllers::ChannelAuthMode;
#[cfg(test)]
pub(crate) use crate::config::Config;
#[cfg(test)]
pub(crate) use connect::merge_listener_health;

// Re-export public ops functions.
pub use connect::{
    channel_status, connect_channel, connected_channel_slugs, disconnect_channel,
    get_default_channel, set_default_channel, test_channel,
};
pub use discord::{discord_check_permissions, discord_list_channels, discord_list_guilds};
pub use messaging::{
    channel_create_thread, channel_list_threads, channel_send_message, channel_send_reaction,
    channel_update_thread,
};

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
