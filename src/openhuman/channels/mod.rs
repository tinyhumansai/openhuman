//! Channel implementations and runtime orchestration.
//!
//! Compile-time gate (`channels`, default-ON — #4801): the whole domain lives
//! behind the feature EXCEPT two dependency-free carve-outs that always-on code
//! reaches directly:
//!
//! * `traits` — a one-line re-export of the `tinychannels` `Channel` /
//!   `SendMessage` traits, named by the always-on agent-harness interactive
//!   loop (`agent::harness::session::runtime::run_interactive`).
//! * `cli` — `CliChannel`, the dependency-free local stdin/stdout REPL the same
//!   interactive loop drives in every build.
//!
//! Everything else (`providers`, `host`, `controllers`, `runtime`, `bus`,
//! `proactive`, `commands`, `context`, `routes`, `relay_runtime`, the provider
//! re-exports, `doctor_channels`, `start_channels`, the `build_system_prompt`
//! re-export and the `test_support` re-export) is `#[cfg(feature = "channels")]`.
//! Nothing INSIDE those submodules changes when the gate flips.
//!
//! The gate sheds ZERO dependencies — `tinychannels` stays load-bearing for the
//! config schema, the `DomainEvent` inbound envelope and security pairing — so
//! its value is compile-time surface + binary size, not the dep tree. See
//! AGENTS.md "channels gate".

// Always-compiled carve-outs (see module docs).
pub mod cli;
pub mod traits;

pub use cli::CliChannel;
pub use traits::{Channel, ChannelSendExt, SendMessage};

#[cfg(feature = "channels")]
pub mod bus;
#[cfg(feature = "channels")]
pub mod controllers;
#[cfg(feature = "channels")]
pub mod host;
#[cfg(feature = "channels")]
pub mod proactive;
#[cfg(feature = "channels")]
pub mod providers;
#[cfg(feature = "channels")]
pub(crate) mod relay_runtime;

#[cfg(feature = "channels")]
mod commands;
#[cfg(feature = "channels")]
pub(crate) mod context;
#[cfg(feature = "channels")]
mod routes;
#[cfg(feature = "channels")]
mod runtime;

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
pub use commands::doctor_channels;
#[cfg(feature = "channels")]
pub use controllers::{ChannelAuthMode, ChannelDefinition};
// Channel system-prompt assembly lives in
// `crate::openhuman::context::channels_prompt` alongside the rest of
// the prompt-building code. Re-exported here for callers that used the
// old `channels::build_system_prompt` path.
#[cfg(feature = "channels")]
pub use crate::openhuman::context::channels_prompt::build_system_prompt;
#[cfg(feature = "channels")]
pub use runtime::start_channels;
