//! Channel implementations and runtime orchestration.

pub mod cli;
pub mod dingtalk;
pub mod discord;
pub mod email_channel;
pub mod imessage;
pub mod irc;
pub mod lark;
pub mod linq;
#[cfg(feature = "channel-matrix")]
pub mod matrix;
pub mod mattermost;
pub mod qq;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod traits;
pub mod whatsapp;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_storage;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_web;

mod commands;
mod context;
mod prompt;
mod routes;
mod runtime;

#[cfg(test)]
mod tests;

pub use cli::CliChannel;
pub use dingtalk::DingTalkChannel;
pub use discord::DiscordChannel;
pub use email_channel::EmailChannel;
pub use imessage::IMessageChannel;
pub use irc::IrcChannel;
pub use lark::LarkChannel;
pub use linq::LinqChannel;
#[cfg(feature = "channel-matrix")]
pub use matrix::MatrixChannel;
pub use mattermost::MattermostChannel;
pub use qq::QQChannel;
pub use signal::SignalChannel;
pub use slack::SlackChannel;
pub use telegram::TelegramChannel;
pub use traits::{Channel, SendMessage};
pub use whatsapp::WhatsAppChannel;
#[cfg(feature = "whatsapp-web")]
pub use whatsapp_web::WhatsAppWebChannel;

pub use commands::doctor_channels;
pub use prompt::build_system_prompt;
pub use runtime::start_channels;
