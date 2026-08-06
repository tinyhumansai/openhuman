//! Shared types for the desktop companion session (shell-side).
//!
//! Migrated from the former core `desktop_companion::types`. The only
//! behavioural change is the state-changed event: it is now delivered to the
//! frontend as a Tauri event with a **camelCase** payload
//! (`{ sessionId, state, previousState }`) instead of the old Socket.IO
//! `companion:state_changed` snake_case payload.

use serde::{Deserialize, Serialize};

/// Visual state of the companion surface, broadcast to the overlay window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompanionState {
    /// No interaction in progress; mascot idles.
    #[default]
    Idle,
    /// Microphone is live — capturing user speech.
    Listening,
    /// Transcript sent to LLM; awaiting response.
    Thinking,
    /// TTS is playing the response audio.
    Speaking,
    /// An unrecoverable error occurred in the current turn.
    Error,
}

impl std::fmt::Display for CompanionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Listening => write!(f, "listening"),
            Self::Thinking => write!(f, "thinking"),
            Self::Speaking => write!(f, "speaking"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// A single conversation turn in the companion session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    /// Who spoke — `"user"` or `"assistant"`.
    pub role: String,
    /// The text content of this turn.
    pub content: String,
    /// Epoch milliseconds when this turn was recorded.
    pub timestamp_ms: i64,
}

/// Persistent configuration for the desktop companion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionConfig {
    /// Hotkey string for activation (e.g. `"ctrl+space"`).
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Activation mode: `"push"` (hold-to-talk) or `"tap"` (toggle).
    #[serde(default = "default_activation_mode")]
    pub activation_mode: String,
    /// Session TTL in seconds. `0` means no automatic expiry.
    #[serde(default = "default_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            activation_mode: default_activation_mode(),
            ttl_secs: default_ttl_secs(),
        }
    }
}

fn default_hotkey() -> String {
    "ctrl+space".into()
}
fn default_activation_mode() -> String {
    "push".into()
}
fn default_ttl_secs() -> u64 {
    3600
}

/// Parameters for starting a companion session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartCompanionSessionParams {
    /// Explicit user consent to audio capture.
    pub consent: bool,
    /// Optional TTL override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
}

/// Parameters for stopping a companion session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopCompanionSessionParams {
    /// Optional reason for stopping (shown in logs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Snapshot of the current companion session status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionSessionStatus {
    pub active: bool,
    pub state: CompanionState,
    pub session_id: Option<String>,
    pub started_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub remaining_ms: Option<i64>,
    pub turn_count: usize,
    pub last_error: Option<String>,
}

/// Result of starting a companion session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartCompanionSessionResult {
    pub session_id: String,
    pub state: CompanionState,
    pub expires_at_ms: Option<i64>,
}

/// Result of stopping a companion session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopCompanionSessionResult {
    pub stopped: bool,
    pub reason: Option<String>,
}

/// Payload of the `companion://state_changed` Tauri event delivered to the
/// frontend. Serialized **camelCase** to match the JS companion slice /
/// overlay contract (`{ sessionId, state, previousState }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStateChangedEvent {
    pub session_id: String,
    pub state: CompanionState,
    pub previous_state: CompanionState,
}
