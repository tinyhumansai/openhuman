//! Channel contracts and OpenHuman runtime orchestration.
//!
//! Provider orchestration is gated by `channels` and runs through the
//! TinyChannels TinyBus module. Serialized configuration, messages, and
//! schemas come from `tinychannels-bus`; OpenHuman retains host policy and
//! lifecycle orchestration.

// Always-compiled carve-outs (see module docs).
pub mod contract_schema;
#[cfg(any(feature = "voice", feature = "channels"))]
pub(crate) mod credentials;

pub use tinychannels_bus::{Channel, ChannelSendExt, SendMessage};

#[cfg(feature = "channels")]
pub mod bus;
#[cfg(feature = "channels")]
mod commands;
#[cfg(feature = "channels")]
pub(crate) use commands::{configured_providers, has_listener_configuration};
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

// Legacy provider paths are retained for unit fixtures only. Production callers
// use the module-backed runtime and the portable bus contracts above.
#[cfg(all(feature = "channels", test))]
pub use providers::dingtalk;
#[cfg(all(feature = "channels", test))]
pub use providers::discord;
#[cfg(all(feature = "channels", test))]
pub use providers::email_channel;
#[cfg(all(feature = "channels", test))]
pub use providers::imessage;
#[cfg(all(feature = "channels", test))]
pub use providers::irc;
#[cfg(all(feature = "channels", test))]
pub use providers::lark;
#[cfg(all(feature = "channels", test))]
pub use providers::linq;
#[cfg(all(feature = "channels", test))]
pub use providers::mattermost;
#[cfg(all(feature = "channels", test))]
pub use providers::qq;
#[cfg(all(feature = "channels", test))]
pub use providers::signal;
#[cfg(all(feature = "channels", test))]
pub use providers::slack;
#[cfg(all(feature = "channels", test))]
pub use providers::telegram;
#[cfg(all(feature = "channels", test))]
pub use providers::whatsapp;
#[cfg(all(feature = "whatsapp-web", test))]
pub use providers::whatsapp_web;
#[cfg(all(feature = "channels", test))]
pub use providers::yuanbao;

#[cfg(all(feature = "channels", test))]
pub use dingtalk::DingTalkChannel;
#[cfg(all(feature = "channels", test))]
pub use discord::DiscordChannel;
#[cfg(all(feature = "channels", test))]
pub use email_channel::EmailChannel;
#[cfg(all(feature = "channels", test))]
pub use imessage::IMessageChannel;
#[cfg(all(feature = "channels", test))]
pub use irc::IrcChannel;
#[cfg(all(feature = "channels", test))]
pub use lark::LarkChannel;
#[cfg(all(feature = "channels", test))]
pub use linq::LinqChannel;
#[cfg(all(feature = "channels", test))]
pub use mattermost::MattermostChannel;
#[cfg(all(feature = "channels", test))]
pub use qq::QQChannel;
#[cfg(all(feature = "channels", test))]
pub use signal::SignalChannel;
#[cfg(all(feature = "channels", test))]
pub use slack::SlackChannel;
#[cfg(all(feature = "channels", test))]
pub use telegram::TelegramChannel;
#[cfg(all(feature = "channels", test))]
pub use whatsapp::WhatsAppChannel;
#[cfg(all(feature = "whatsapp-web", test))]
pub use whatsapp_web::WhatsAppWebChannel;
#[cfg(all(feature = "channels", test))]
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

#[cfg(all(test, feature = "channels", feature = "modules"))]
#[path = "module_fixture_tests.rs"]
pub(crate) mod module_fixture;

#[cfg(test)]
#[path = "contract_reexport_tests.rs"]
mod contract_reexport_tests;
