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

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;
