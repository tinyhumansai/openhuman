use serde::{Deserialize, Serialize};

/// Payload for `start_listening` — no arguments needed at the JS boundary.
#[derive(Debug, Serialize, Deserialize)]
pub struct StartListeningRequest {}

/// Returned by `stop_listening` once the recognizer finalizes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    /// Final transcript text (may be empty if nothing was recognized).
    pub text: String,
    /// Always true when returned from `stop_listening`.
    pub is_final: bool,
}

/// Args for the `speak` command.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakRequest {
    pub text: String,
    /// Optional BCP-47 voice identifier (e.g. `"com.apple.voice.compact.en-US.Samantha"`).
    pub voice_id: Option<String>,
    /// Speech rate multiplier: 0.5 (slow) to 2.0 (fast). Default = 1.0.
    pub rate: Option<f32>,
}

/// Describes a single on-device TTS voice.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    /// AVSpeechSynthesisVoice.identifier
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// BCP-47 language tag, e.g. "en-US".
    pub lang: String,
}

// ---------------------------------------------------------------------------
// Event payloads (emitted over the Tauri event bus)
// ---------------------------------------------------------------------------

/// `ptt://transcript-partial` — live partial result while recording.
#[derive(Debug, Serialize, Deserialize)]
pub struct TranscriptPartialPayload {
    pub text: String,
}

/// `ptt://transcript-final` — final result after `stop_listening`.
#[derive(Debug, Serialize, Deserialize)]
pub struct TranscriptFinalPayload {
    pub text: String,
}

/// `ptt://tts-started`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStartedPayload {
    pub utterance_id: String,
}

/// `ptt://tts-ended`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsEndedPayload {
    pub utterance_id: String,
    /// false if cancelled before completion.
    pub finished: bool,
}

/// `ptt://error` — async audio / permission errors.
#[derive(Debug, Serialize, Deserialize)]
pub struct PttErrorPayload {
    pub code: String,
    pub message: String,
}
