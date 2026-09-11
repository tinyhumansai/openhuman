//! Serializable DTOs for voice domain RPC responses.

use serde::{Deserialize, Serialize};

use crate::openhuman::inference::{LocalAiSpeechResult, LocalAiTtsResult};

/// Result of a speech-to-text transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSpeechResult {
    /// Final text — cleaned by LLM post-processing when available,
    /// otherwise identical to `raw_text`.
    pub text: String,
    /// Raw engine output before LLM cleanup.
    pub raw_text: String,
    pub model_id: String,
}

/// Result of a text-to-speech synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceTtsResult {
    pub output_path: String,
    pub voice_id: String,
}

/// Proactive availability check for STT/TTS binaries and models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceStatus {
    pub stt_available: bool,
    pub tts_available: bool,
    pub stt_model_id: String,
    pub tts_voice_id: String,
    pub piper_binary: Option<String>,
    pub tts_voice_path: Option<String>,
    /// Whether LLM post-processing is enabled for transcription cleanup.
    pub llm_cleanup_enabled: bool,
    /// Resolved STT routing string — `"cloud"` for the backend proxy, or the
    /// third-party slug selected by `voice_server.stt_engine`. Echoed so the
    /// settings panel can render the picker without an extra RPC.
    #[serde(default)]
    pub stt_engine: String,
    /// Why `stt_available` is false, when it is (e.g. the selected engine has
    /// no `voice_providers` entry). `None` when STT is usable.
    #[serde(default)]
    pub stt_error: Option<String>,
    /// Currently selected TTS provider ("cloud" or "piper").
    #[serde(default)]
    pub tts_provider: String,
}

impl From<LocalAiSpeechResult> for VoiceSpeechResult {
    fn from(r: LocalAiSpeechResult) -> Self {
        Self {
            text: r.text.clone(),
            raw_text: r.text,
            model_id: r.model_id,
        }
    }
}

impl From<LocalAiTtsResult> for VoiceTtsResult {
    fn from(r: LocalAiTtsResult) -> Self {
        Self {
            output_path: r.output_path,
            voice_id: r.voice_id,
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
