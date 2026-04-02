//! Serializable DTOs for voice domain RPC responses.

use serde::{Deserialize, Serialize};

use crate::openhuman::local_ai::{LocalAiSpeechResult, LocalAiTtsResult};

/// Result of a speech-to-text transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSpeechResult {
    /// Final text — cleaned by LLM post-processing when available,
    /// otherwise identical to `raw_text`.
    pub text: String,
    /// Raw whisper output before LLM cleanup.
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
    pub whisper_binary: Option<String>,
    pub piper_binary: Option<String>,
    pub stt_model_path: Option<String>,
    pub tts_voice_path: Option<String>,
    /// Whether the whisper model is loaded in-process (low-latency mode).
    pub whisper_in_process: bool,
    /// Whether LLM post-processing is enabled for transcription cleanup.
    pub llm_cleanup_enabled: bool,
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
mod tests {
    use super::*;

    #[test]
    fn voice_speech_result_serializes_correctly() {
        let r = VoiceSpeechResult {
            text: "hello world".into(),
            raw_text: "hello world um".into(),
            model_id: "ggml-tiny-q5_1.bin".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["text"], "hello world");
        assert_eq!(v["raw_text"], "hello world um");
        assert_eq!(v["model_id"], "ggml-tiny-q5_1.bin");
    }

    #[test]
    fn voice_tts_result_serializes_correctly() {
        let r = VoiceTtsResult {
            output_path: "/tmp/out.wav".into(),
            voice_id: "en_US-lessac-medium".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["output_path"], "/tmp/out.wav");
        assert_eq!(v["voice_id"], "en_US-lessac-medium");
    }

    #[test]
    fn voice_status_serializes_correctly() {
        let s = VoiceStatus {
            stt_available: true,
            tts_available: false,
            stt_model_id: "tiny.bin".into(),
            tts_voice_id: "en_US-lessac-medium".into(),
            whisper_binary: Some("/usr/local/bin/whisper-cli".into()),
            piper_binary: None,
            stt_model_path: Some("/models/stt/tiny.bin".into()),
            tts_voice_path: None,
            whisper_in_process: true,
            llm_cleanup_enabled: true,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["stt_available"], true);
        assert_eq!(v["tts_available"], false);
        assert!(v["piper_binary"].is_null());
        assert_eq!(v["whisper_in_process"], true);
        assert_eq!(v["llm_cleanup_enabled"], true);
    }

    #[test]
    fn from_local_ai_speech_result() {
        let local = LocalAiSpeechResult {
            text: "test".into(),
            model_id: "tiny".into(),
        };
        let voice: VoiceSpeechResult = local.into();
        assert_eq!(voice.text, "test");
        assert_eq!(voice.raw_text, "test");
        assert_eq!(voice.model_id, "tiny");
    }

    #[test]
    fn from_local_ai_tts_result() {
        let local = LocalAiTtsResult {
            output_path: "/out.wav".into(),
            voice_id: "voice1".into(),
        };
        let voice: VoiceTtsResult = local.into();
        assert_eq!(voice.output_path, "/out.wav");
        assert_eq!(voice.voice_id, "voice1");
    }

    #[test]
    fn serde_round_trip_speech_result() {
        let original = VoiceSpeechResult {
            text: "round trip".into(),
            raw_text: "round trip uh".into(),
            model_id: "model".into(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: VoiceSpeechResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.text, original.text);
        assert_eq!(decoded.raw_text, original.raw_text);
        assert_eq!(decoded.model_id, original.model_id);
    }
}
