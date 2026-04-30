//! Telegram channel — long-polls the Bot API for updates.

mod attachments;
mod channel;
mod channel_core;
mod channel_ops;
mod channel_recv;
mod channel_send;
mod channel_types;
mod text;

pub use channel_types::TelegramChannel;
