//! Wire and config types for the voice server: run state, status snapshot,
//! and the tunables that shape a recording → transcription pass.

use tinyvoice_bus::hotkey::ActivationMode;

/// Running state of the voice server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// Server is not running.
    Stopped,
    /// Server is running and idle, waiting for hotkey.
    Idle,
    /// Actively recording audio.
    Recording,
    /// Transcribing recorded audio.
    Transcribing,
}

impl From<crate::modules::voice::SessionPhase> for ServerState {
    fn from(phase: crate::modules::voice::SessionPhase) -> Self {
        use crate::modules::voice::SessionPhase;
        match phase {
            SessionPhase::Stopped => Self::Stopped,
            SessionPhase::Idle => Self::Idle,
            SessionPhase::Recording => Self::Recording,
            SessionPhase::Transcribing => Self::Transcribing,
        }
    }
}

/// Status snapshot of the voice server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VoiceServerStatus {
    pub state: ServerState,
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    pub transcription_count: u64,
    pub last_error: Option<String>,
}

/// Default silence threshold (RMS energy). Recordings with peak RMS below
/// this are considered silent and skipped. Matches OpenWhispr's 0.002 default.
pub(super) const DEFAULT_SILENCE_THRESHOLD: f32 = 0.002;

/// Maximum number of recent transcriptions to keep as context for the STT engine's
/// initial_prompt, improving continuity across consecutive recordings.
pub(super) const MAX_RECENT_TRANSCRIPTS: usize = 5;

/// Maximum character length of the combined initial prompt (dictionary +
/// recent transcripts). The prompt token budget is limited.
pub(super) const MAX_INITIAL_PROMPT_CHARS: usize = 500;

/// Configuration for the voice server.
#[derive(Debug, Clone)]
pub struct VoiceServerConfig {
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    /// Skip LLM post-processing on transcriptions.
    pub skip_cleanup: bool,
    /// Optional conversation context for better transcription accuracy.
    pub context: Option<String>,
    /// Minimum recording duration in seconds. Shorter recordings are discarded.
    pub min_duration_secs: f32,
    /// RMS energy threshold for silence detection. Recordings with peak
    /// energy below this are treated as silence and skipped.
    pub silence_threshold: f32,
    /// Custom vocabulary words to bias the STT engine toward (passed as initial_prompt).
    pub custom_dictionary: Vec<String>,
}

impl Default for VoiceServerConfig {
    fn default() -> Self {
        Self {
            hotkey: "Fn".to_string(),
            activation_mode: ActivationMode::Push,
            skip_cleanup: false,
            context: None,
            min_duration_secs: 0.3,
            silence_threshold: DEFAULT_SILENCE_THRESHOLD,
            custom_dictionary: Vec::new(),
        }
    }
}
