//! Spoken-reply policy for channels (`[channel_voice]`).
//!
//! The host resolves one [`VoiceReplyMode`] per turn: a per-thread override
//! wins over a per-channel override, which wins over the global default.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// When a channel reply is also spoken.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceReplyMode {
    /// Speak only when the inbound message was a voice note.
    #[default]
    Mirror,
    /// Always speak the reply.
    Always,
    /// Never speak the reply.
    Never,
    /// Speak only when the request explicitly asked for a spoken reply.
    SpeakReply,
}

/// Per-channel overrides; unset fields inherit the global value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct ChannelVoiceOverride {
    pub reply_mode: Option<VoiceReplyMode>,
    pub voice: Option<String>,
    pub max_outbound_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct ChannelVoiceConfig {
    /// Global reply mode.
    pub reply_mode: VoiceReplyMode,
    /// Default synthesis voice; `None` uses the backend default.
    pub voice: Option<String>,
    /// Longest spoken reply the host will synthesize, in seconds.
    pub max_outbound_secs: u32,
    /// Overrides keyed by channel name (for example `telegram`).
    pub channels: BTreeMap<String, ChannelVoiceOverride>,
    /// Reply-mode overrides keyed by conversation (thread) id.
    pub threads: BTreeMap<String, VoiceReplyMode>,
}

impl Default for ChannelVoiceConfig {
    fn default() -> Self {
        Self {
            reply_mode: VoiceReplyMode::Mirror,
            voice: None,
            max_outbound_secs: 120,
            channels: BTreeMap::new(),
            threads: BTreeMap::new(),
        }
    }
}

impl ChannelVoiceConfig {
    /// Thread override, then channel override, then the global mode.
    pub fn resolve_mode(&self, channel: &str, thread: &str) -> VoiceReplyMode {
        self.threads
            .get(thread)
            .copied()
            .or_else(|| self.channels.get(channel).and_then(|c| c.reply_mode))
            .unwrap_or(self.reply_mode)
    }

    /// Channel voice, falling back to the global voice.
    pub fn resolve_voice(&self, channel: &str) -> Option<&str> {
        self.channels
            .get(channel)
            .and_then(|c| c.voice.as_deref())
            .or(self.voice.as_deref())
    }

    /// Channel outbound cap, falling back to the global cap.
    pub fn resolve_max_outbound_secs(&self, channel: &str) -> u32 {
        self.channels
            .get(channel)
            .and_then(|c| c.max_outbound_secs)
            .unwrap_or(self.max_outbound_secs)
    }
}

#[cfg(test)]
#[path = "channel_voice_tests.rs"]
mod tests;
