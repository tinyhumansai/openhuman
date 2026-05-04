//! Telegram Bot API channel implementation.
//!
//! This module is the orchestration entry point for the Telegram channel.
//! Implementation is split across sibling modules by concern:
//!
//! - [`super::channel_types`]  — struct definition and private helper types
//! - [`super::channel_core`]   — constructor, config, pairing/auth, API plumbing
//! - [`super::channel_recv`]   — inbound parsing, allowlist checks, mention filtering
//! - [`super::channel_send`]   — outbound text, media, reactions, attachments
//! - [`super::channel_ops`]    — `Channel` trait impl (send/listen/draft/typing)

// Re-export so that the `#[path = "channel_tests.rs"]` test module can reach
// `TelegramChannel` via `super::TelegramChannel`.
pub use super::channel_types::TelegramChannel;
#[cfg(test)]
pub(super) use super::channel_types::TelegramTypingTask;

#[cfg(test)]
#[path = "channel_tests.rs"]
mod tests;
