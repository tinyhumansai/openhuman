//! Types for the overlay attention bus.

use serde::{Deserialize, Serialize};

/// Visual tone hint for the overlay bubble. The frontend maps these to
/// bubble colours (see `OverlayApp.tsx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OverlayAttentionTone {
    /// Informational / neutral (slate bubble).
    #[default]
    Neutral,
    /// Important / assistant-initiated (blue bubble).
    Accent,
    /// Positive confirmation (green bubble).
    Success,
}

/// A single attention message emitted toward the overlay window.
///
/// Only `message` is required. All other fields have sensible defaults
/// so callers can do `OverlayAttentionEvent::new("Hey …")` and go.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayAttentionEvent {
    /// Stable id for this message; if `None`, the frontend generates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The text to display. The overlay types it out character by
    /// character, so keep it short (a sentence or two).
    pub message: String,
    /// Visual tone for the bubble.
    #[serde(default)]
    pub tone: OverlayAttentionTone,
    /// How long the overlay should stay visible, in milliseconds, before
    /// auto-dismissing back to idle. `None` → frontend default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u32>,
    /// Free-form source label for logging / debugging ("subconscious",
    /// "heartbeat", "screen_intelligence", …). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl OverlayAttentionEvent {
    /// Convenience constructor with neutral tone and default ttl.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            id: None,
            message: message.into(),
            tone: OverlayAttentionTone::default(),
            ttl_ms: None,
            source: None,
        }
    }

    /// Builder-style source setter for diagnostics.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Builder-style tone setter.
    pub fn with_tone(mut self, tone: OverlayAttentionTone) -> Self {
        self.tone = tone;
        self
    }

    /// Builder-style ttl setter.
    pub fn with_ttl_ms(mut self, ttl_ms: u32) -> Self {
        self.ttl_ms = Some(ttl_ms);
        self
    }
}
