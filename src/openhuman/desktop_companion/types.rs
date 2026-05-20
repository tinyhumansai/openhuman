//! Shared types for the desktop companion session.

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
    /// Transcript + screen context sent to LLM; awaiting response.
    Thinking,
    /// TTS is playing the response audio.
    Speaking,
    /// Visual pointer is animating toward a UI target.
    Pointing,
    /// An unrecoverable error occurred in the current turn.
    Error,
}

impl CompanionState {
    /// Returns `true` for states that represent an active interaction turn.
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Error)
    }
}

impl std::fmt::Display for CompanionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Listening => write!(f, "listening"),
            Self::Thinking => write!(f, "thinking"),
            Self::Speaking => write!(f, "speaking"),
            Self::Pointing => write!(f, "pointing"),
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
    /// Whether to capture a screenshot on each activation.
    #[serde(default = "default_true")]
    pub capture_screen: bool,
    /// Whether to include the foreground app context.
    #[serde(default = "default_true")]
    pub include_app_context: bool,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            activation_mode: default_activation_mode(),
            ttl_secs: default_ttl_secs(),
            capture_screen: true,
            include_app_context: true,
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
fn default_true() -> bool {
    true
}

/// Parameters for starting a companion session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartCompanionSessionParams {
    /// Explicit user consent to screen monitoring and audio capture.
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

/// Event emitted when companion state changes (for Socket.IO bridge).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionStateChangedEvent {
    pub session_id: String,
    pub state: CompanionState,
    pub previous_state: CompanionState,
    /// Optional human-readable message (e.g. error details, response text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
