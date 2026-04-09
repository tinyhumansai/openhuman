//! Voice dictation configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Activation mode for the dictation hotkey.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DictationActivationMode {
    /// Press once to start, press again to stop.
    Toggle,
    /// Hold to record, release to stop (push-to-talk).
    #[default]
    Push,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DictationConfig {
    /// Whether voice dictation is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Global hotkey for activating dictation (e.g. "Fn").
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    /// Activation mode: "toggle" (press to start/stop) or "push" (hold to record).
    #[serde(default)]
    pub activation_mode: DictationActivationMode,

    /// Whether to refine raw transcription through a local LLM for grammar/punctuation.
    #[serde(default = "default_llm_refinement")]
    pub llm_refinement: bool,

    /// Whether to use WebSocket streaming transcription (chunks sent in real-time)
    /// instead of batch transcription after recording stops.
    #[serde(default = "default_streaming")]
    pub streaming: bool,

    /// Interval in milliseconds between streaming inference passes on accumulated audio.
    #[serde(default = "default_streaming_interval_ms")]
    pub streaming_interval_ms: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_hotkey() -> String {
    "Fn".to_string()
}

fn default_llm_refinement() -> bool {
    true
}

fn default_streaming() -> bool {
    true
}

fn default_streaming_interval_ms() -> u64 {
    2000
}

impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            hotkey: default_hotkey(),
            activation_mode: DictationActivationMode::default(),
            llm_refinement: default_llm_refinement(),
            streaming: default_streaming(),
            streaming_interval_ms: default_streaming_interval_ms(),
        }
    }
}
