//! Core channel connect/disconnect/status operations.

mod connect_channel;
mod disconnect;
mod email;
mod memory;
pub(super) mod shared;
mod status;
mod test_channel;

pub use connect_channel::connect_channel;
#[cfg(test)]
pub(crate) use connect_channel::persist_prepared_connection;
pub use disconnect::disconnect_channel;
#[cfg(test)]
pub(crate) use email::persist_email_config;
#[cfg(test)]
pub(crate) use shared::merge_listener_health;
pub use status::{
    channel_status, connected_channel_slugs, get_default_channel, set_default_channel,
};
pub use test_channel::test_channel;
