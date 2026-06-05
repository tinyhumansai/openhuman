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
#[serde(default)]
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

    /// Phase 2 — always-on listening. When true, the voice server keeps the
    /// microphone open continuously and segments utterances with
    /// voice-activity detection (VAD) instead of requiring a hotkey press.
    /// Off by default: always-on listening has obvious privacy weight, so it
    /// is strictly opt-in.
    #[serde(default)]
    pub always_on_enabled: bool,

    /// VAD speech-onset threshold (peak RMS energy). A frame whose RMS rises
    /// above this is treated as the start of speech. Slightly higher than the
    /// hotkey `silence_threshold` because an always-open mic must reject more
    /// ambient noise before opening an utterance.
    #[serde(default = "default_vad_onset_threshold")]
    pub vad_onset_threshold: f32,

    /// VAD hangover: how long (milliseconds) RMS must stay below the onset
    /// threshold before the current utterance is considered finished. Prevents
    /// chopping an utterance on natural mid-sentence pauses.
    #[serde(default = "default_vad_hangover_ms")]
    pub vad_hangover_ms: u32,

    /// Minimum speech duration (milliseconds) for a segment to be emitted.
    /// Shorter blips (a cough, a door) are discarded before transcription.
    #[serde(default = "default_vad_min_speech_ms")]
    pub vad_min_speech_ms: u32,

    /// Hard ceiling (seconds) on a single always-on utterance. Forces a flush
    /// so a continuous noise source can't grow an unbounded recording.
    #[serde(default = "default_vad_max_utterance_secs")]
    pub vad_max_utterance_secs: f32,

    /// Wake word for always-on mode. An utterance is only delivered to the agent
    /// when its transcript contains this phrase; the phrase is stripped and the
    /// remainder is sent as the command. Empty = no wake word (deliver every
    /// utterance). Default "Hey Tiny".
    #[serde(default = "default_wake_word")]
    pub wake_word: String,
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

fn default_vad_onset_threshold() -> f32 {
    0.01
}

fn default_vad_hangover_ms() -> u32 {
    800
}

fn default_vad_min_speech_ms() -> u32 {
    300
}

fn default_vad_max_utterance_secs() -> f32 {
    30.0
}

fn default_wake_word() -> String {
    "Hey Tiny".to_string()
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
            always_on_enabled: false,
            vad_onset_threshold: default_vad_onset_threshold(),
            vad_hangover_ms: default_vad_hangover_ms(),
            vad_min_speech_ms: default_vad_min_speech_ms(),
            vad_max_utterance_secs: default_vad_max_utterance_secs(),
            wake_word: default_wake_word(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_opt_in_and_sane() {
        let c = VoiceServerConfig::default();
        // Always-on is privacy-sensitive — must default off.
        assert!(!c.always_on_enabled);
        // Onset must sit above the hotkey silence floor so an open mic rejects
        // ambient noise that the push-to-talk path would have tolerated.
        assert!(c.vad_onset_threshold > c.silence_threshold);
        assert!(c.vad_hangover_ms > 0);
        assert!(c.vad_min_speech_ms > 0);
        assert!(c.vad_max_utterance_secs > 0.0);
    }

    #[test]
    fn deserializes_with_all_vad_fields_defaulted() {
        // An older config file with none of the Phase 2 keys must still load.
        let c: VoiceServerConfig = serde_json::from_str("{}").unwrap();
        assert!(!c.always_on_enabled);
        assert_eq!(c.vad_hangover_ms, default_vad_hangover_ms());
        assert_eq!(c.vad_min_speech_ms, default_vad_min_speech_ms());
    }
}
