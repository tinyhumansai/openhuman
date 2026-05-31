//! Telegram channel — long-polls the Bot API for updates.

mod attachments;
mod bus;
mod channel;
mod channel_core;
mod channel_ops;
mod channel_recv;
mod channel_send;
mod channel_types;
pub mod remote_control;
mod session_store;
mod text;

pub use bus::TelegramRemoteSubscriber;
pub use channel_types::TelegramChannel;
pub use remote_control::TelegramRemoteCommand;

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    //! Debug-build seams for raw integration coverage of Telegram send helpers.

    use super::TelegramChannel;

    pub fn parse_reaction_marker_for_test(content: &str) -> (String, Option<String>) {
        TelegramChannel::parse_reaction_marker(content)
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;
