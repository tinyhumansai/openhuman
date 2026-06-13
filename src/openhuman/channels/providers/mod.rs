//! External channel backends (Telegram, Signal, WhatsApp, Slack, Matrix, …).

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
// Public (like every sibling provider module) so cross-module callers reach it
// in *all* profiles. It was previously `pub` only under test/debug and private
// in release, which compiled in debug but broke the release build once a
// cross-module caller appeared (`agent::task_dispatcher` →
// `presentation::deliver_response`): release-only `E0603: module is private`.
pub mod presentation;
pub mod qq;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod web;
pub mod whatsapp;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_web;
pub mod yuanbao;
