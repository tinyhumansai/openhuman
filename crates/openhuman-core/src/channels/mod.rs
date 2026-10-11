//! Channel contracts and OpenHuman runtime orchestration.
//!
//! Provider orchestration is gated by `channels`. Serialized configuration,
//! messages, and schemas come from `tinychannels-bus`.
//!
//! The local REPL is host orchestration. The legacy provider trait remains an
//! explicit implementation exception during the runtime migration to TinyBus.

// Always-compiled carve-outs (see module docs).
pub mod contract_schema;
#[cfg(any(feature = "voice", feature = "channels"))]
pub(crate) mod credentials;
pub mod traits;

pub use traits::{Channel, ChannelSendExt, SendMessage};

#[cfg(feature = "channels")]
pub mod bus;
#[cfg(feature = "channels")]
mod commands;
#[cfg(feature = "channels")]
pub(crate) mod context;
#[cfg(feature = "channels")]
pub mod controllers;
#[cfg(feature = "channels")]
pub mod host;
#[cfg(feature = "channels")]
pub mod proactive;
#[cfg(feature = "channels")]
pub mod providers;
#[cfg(feature = "channels")]
mod routes;
#[cfg(feature = "channels")]
mod runtime;
#[cfg(feature = "channels")]
pub(crate) mod system_prompt;

#[cfg(feature = "channels")]
pub(crate) use system_prompt::ChannelSystemPrompt;

#[cfg(all(feature = "channels", test))]
mod tests;

// Stable `channels::<provider>` paths (implementation lives under `providers/`).
#[cfg(feature = "channels")]
pub use providers::dingtalk;
#[cfg(feature = "channels")]
pub use providers::discord;
#[cfg(feature = "channels")]
pub use providers::email_channel;
#[cfg(feature = "channels")]
pub use providers::imessage;
#[cfg(feature = "channels")]
pub use providers::irc;
#[cfg(feature = "channels")]
pub use providers::lark;
#[cfg(feature = "channels")]
pub use providers::linq;
#[cfg(feature = "channels")]
pub use providers::mattermost;
#[cfg(feature = "channels")]
pub use providers::qq;
#[cfg(feature = "channels")]
pub use providers::signal;
#[cfg(feature = "channels")]
pub use providers::slack;
#[cfg(feature = "channels")]
pub use providers::telegram;
#[cfg(feature = "channels")]
pub use providers::whatsapp;
#[cfg(feature = "whatsapp-web")]
pub use providers::whatsapp_web;
#[cfg(feature = "channels")]
pub use providers::yuanbao;

#[cfg(feature = "channels")]
pub use dingtalk::DingTalkChannel;
#[cfg(feature = "channels")]
pub use discord::DiscordChannel;
#[cfg(feature = "channels")]
pub use email_channel::EmailChannel;
#[cfg(feature = "channels")]
pub use imessage::IMessageChannel;
#[cfg(feature = "channels")]
pub use irc::IrcChannel;
#[cfg(feature = "channels")]
pub use lark::LarkChannel;
#[cfg(feature = "channels")]
pub use linq::LinqChannel;
#[cfg(feature = "channels")]
pub use mattermost::MattermostChannel;
#[cfg(feature = "channels")]
pub use qq::QQChannel;
#[cfg(feature = "channels")]
pub use signal::SignalChannel;
#[cfg(feature = "channels")]
pub use slack::SlackChannel;
#[cfg(feature = "channels")]
pub use telegram::TelegramChannel;
#[cfg(feature = "channels")]
pub use whatsapp::WhatsAppChannel;
#[cfg(feature = "whatsapp-web")]
pub use whatsapp_web::WhatsAppWebChannel;
#[cfg(feature = "channels")]
pub use yuanbao::YuanbaoChannel;

#[cfg(all(feature = "channels", any(test, debug_assertions)))]
pub use runtime::test_support;

#[cfg(feature = "channels")]
// Channel system-prompt assembly lives in
// `crate::agent::context::channels_prompt` alongside the rest of
// the prompt-building code. Re-exported here for callers that used the
// old `channels::build_system_prompt` path.
#[cfg(feature = "channels")]
pub use crate::agent::context::channels_prompt::build_system_prompt;
#[cfg(feature = "channels")]
pub use commands::doctor_channels;
#[cfg(feature = "channels")]
pub use runtime::start_channels;
#[cfg(feature = "channels")]
pub(crate) use runtime::{session, start_channels_with_session};
