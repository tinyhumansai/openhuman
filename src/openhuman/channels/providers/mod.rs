//! External channel backends (Telegram, Signal, WhatsApp, Slack, …).

pub mod dingtalk;
pub mod discord;
pub mod email_channel;
pub mod imessage;
pub mod irc;
pub mod lark;
pub mod linq;
pub mod mattermost;
pub mod qq;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod whatsapp;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_web;
pub mod yuanbao;
