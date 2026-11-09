//! `[inbound_debounce]` — batching of inbound channel messages per
//! conversation before an agent turn runs.
//!
//! Messages from one sender in one chat that arrive within the quiet window
//! become one turn, and a conversation never runs two turns at once.
//! `[inbound_debounce.by_channel.<name>]` overrides the windows for one
//! channel; `enabled = false` turns merging off but keeps per-conversation
//! ordering.
//!
//! | setting | override |
//! | --- | --- |
//! | `enabled` | `OPENHUMAN_INBOUND_DEBOUNCE_ENABLED` |
//! | `debounce_ms` | `OPENHUMAN_INBOUND_DEBOUNCE_MS` |
//! | `media_debounce_ms` | `OPENHUMAN_INBOUND_DEBOUNCE_MEDIA_MS` |
//! | `max_wait_ms` | `OPENHUMAN_INBOUND_DEBOUNCE_MAX_WAIT_MS` |

use crate::inbound_debounce::DebounceSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

pub const DEFAULT_DEBOUNCE_MS: u64 = 1_500;
pub const DEFAULT_MEDIA_DEBOUNCE_MS: u64 = 4_000;
pub const DEFAULT_MAX_WAIT_MS: u64 = 10_000;
pub const DEFAULT_MAX_BATCH_MESSAGES: usize = 12;
pub const DEFAULT_MAX_BATCH_CHARS: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InboundDebounceConfig {
    /// Merge messages that arrive within the quiet window into one turn.
    pub enabled: bool,
    /// Quiet window after the latest text message, in milliseconds.
    pub debounce_ms: u64,
    /// Quiet window once the batch holds an image or file, in milliseconds.
    pub media_debounce_ms: u64,
    /// Longest a batch waits from its first message, in milliseconds. Raised
    /// to at least the longer of the two windows.
    pub max_wait_ms: u64,
    /// Messages per batch; one more starts a new batch.
    pub max_batch_messages: usize,
    /// Characters per batch; a message that would pass it starts a new batch.
    pub max_batch_chars: usize,
    /// Per-channel overrides keyed by channel name (`telegram`, `discord`, ...).
    pub by_channel: BTreeMap<String, InboundDebounceOverride>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InboundDebounceOverride {
    pub enabled: Option<bool>,
    pub debounce_ms: Option<u64>,
    pub media_debounce_ms: Option<u64>,
    pub max_wait_ms: Option<u64>,
}

impl Default for InboundDebounceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            media_debounce_ms: DEFAULT_MEDIA_DEBOUNCE_MS,
            max_wait_ms: DEFAULT_MAX_WAIT_MS,
            max_batch_messages: DEFAULT_MAX_BATCH_MESSAGES,
            max_batch_chars: DEFAULT_MAX_BATCH_CHARS,
            by_channel: BTreeMap::new(),
        }
    }
}

impl InboundDebounceConfig {
    /// Merging switched off everywhere: one turn per message, still in order.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// The settings for `channel`. A suffix after `:` (an account or
    /// instance) is ignored when looking up the override.
    pub fn settings_for(&self, channel: &str) -> DebounceSettings {
        let name = channel.split(':').next().unwrap_or(channel);
        let over = self.by_channel.get(name);
        let enabled = over.and_then(|o| o.enabled).unwrap_or(self.enabled);
        if !enabled {
            return DebounceSettings::immediate();
        }
        let pick = |field: fn(&InboundDebounceOverride) -> Option<u64>, global: u64| {
            Duration::from_millis(over.and_then(field).unwrap_or(global))
        };
        let debounce = pick(|o| o.debounce_ms, self.debounce_ms);
        let media_debounce = pick(|o| o.media_debounce_ms, self.media_debounce_ms);
        let max_wait = pick(|o| o.max_wait_ms, self.max_wait_ms)
            .max(debounce)
            .max(media_debounce);
        DebounceSettings {
            debounce,
            media_debounce,
            max_wait,
            max_batch_messages: self.max_batch_messages.max(1),
            max_batch_chars: self.max_batch_chars.max(1),
        }
    }
}

#[cfg(test)]
#[path = "inbound_debounce_tests.rs"]
mod tests;
