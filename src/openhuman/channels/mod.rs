//! Channel implementations and runtime orchestration.

pub mod bus;
pub mod cli;
pub mod controllers;
pub mod providers;
pub mod traits;

mod commands;
pub(crate) mod context;
mod prompt;
mod routes;
mod runtime;

#[cfg(test)]
mod tests;

// Stable `channels::<provider>` paths (implementation lives under `providers/`).
pub use providers::dingtalk;
pub use providers::discord;
pub use providers::email_channel;
pub use providers::imessage;
pub use providers::irc;
pub use providers::lark;
pub use providers::linq;
#[cfg(feature = "channel-matrix")]
pub use providers::matrix;
pub use providers::mattermost;
pub use providers::qq;
pub use providers::signal;
pub use providers::slack;
pub use providers::telegram;
pub use providers::web;
pub use providers::whatsapp;
#[cfg(feature = "whatsapp-web")]
pub use providers::whatsapp_storage;
#[cfg(feature = "whatsapp-web")]
pub use providers::whatsapp_web;

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
pub use controllers::{ChannelAuthMode, ChannelDefinition};
pub use prompt::build_system_prompt;
pub use runtime::start_channels;
