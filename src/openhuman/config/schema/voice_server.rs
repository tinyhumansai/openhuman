//! Voice server configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Activation mode for the voice server hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum VoiceActivationMode {
    /// Single press toggles recording on/off.
    Tap,
    /// Hold to record, release to stop.
    #[default]
    Push,
}

/// Configuration for the voice dictation server.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VoiceServerConfig {
    /// Whether the voice server should start automatically with the core.
    #[serde(default)]
    pub auto_start: bool,

    /// Hotkey combination to trigger recording (e.g. "Fn").
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    /// Activation mode: "tap" (toggle) or "push" (hold-to-record).
    #[serde(default)]
    pub activation_mode: VoiceActivationMode,

    /// Skip LLM post-processing for transcriptions.
    /// Default: false (cleanup enabled — matches OpenWhispr behavior).
    #[serde(default)]
    pub skip_cleanup: bool,

    /// Minimum recording duration in seconds. Recordings shorter than
    /// this are discarded.
    #[serde(default = "default_min_duration")]
    pub min_duration_secs: f32,

    /// RMS energy threshold for silence detection. Recordings with peak
    /// energy below this value are treated as silence and skipped without
    /// sending to whisper, preventing hallucinated output.
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold: f32,

    /// Custom dictionary words to bias whisper toward. These are passed
    /// as the `initial_prompt` parameter, improving recognition of names,
    /// technical terms, and domain-specific vocabulary.
    #[serde(default)]
    pub custom_dictionary: Vec<String>,
}

fn default_hotkey() -> String {
    "Fn".to_string()
}

fn default_min_duration() -> f32 {
    0.3
}

fn default_silence_threshold() -> f32 {
    0.002
}

impl Default for VoiceServerConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            hotkey: default_hotkey(),
            activation_mode: VoiceActivationMode::default(),
            skip_cleanup: false,
            min_duration_secs: default_min_duration(),
            silence_threshold: default_silence_threshold(),
            custom_dictionary: Vec::new(),
        }
    }
}
