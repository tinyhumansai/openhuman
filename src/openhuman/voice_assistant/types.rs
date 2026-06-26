//! Request / response types for the `voice_assistant` domain.
//!
//! The voice assistant provides a standalone, local-first voice session
//! (mic → STT → LLM → TTS → speaker) exposed through the controller
//! registry. Audio crosses the RPC boundary as base64-encoded PCM16LE
//! @ 16 kHz mono.

use serde::{Deserialize, Serialize};

/// Inputs to `openhuman.voice_assistant_start_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct StartSessionRequest {
    /// Optional session id; auto-generated when omitted.
    #[serde(default)]
    pub session_id: Option<String>,
    /// STT provider override (`"whisper"` or `"cloud"`). Default: `"whisper"`.
    #[serde(default = "default_stt_provider")]
    pub stt_provider: String,
    /// TTS provider override (`"piper"` or `"cloud"`). Default: `"piper"`.
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,
    /// BCP-47 language hint for STT (e.g. `"en"`).
    #[serde(default)]
    pub language: Option<String>,
}

fn default_stt_provider() -> String {
    "whisper".to_string()
}

fn default_tts_provider() -> String {
    "piper".to_string()
}

/// Outputs from `openhuman.voice_assistant_start_session`.
#[derive(Debug, Clone, Serialize)]
pub struct StartSessionResponse {
    pub ok: bool,
    pub session_id: String,
    pub stt_provider: String,
    pub tts_provider: String,
}

/// Inputs to `openhuman.voice_assistant_push_audio`.
#[derive(Debug, Clone, Deserialize)]
pub struct PushAudioRequest {
    pub session_id: String,
    /// Base64-encoded PCM16LE samples at 16 kHz mono.
    pub pcm_base64: String,
}

/// Outputs from `openhuman.voice_assistant_push_audio`.
#[derive(Debug, Clone, Serialize)]
pub struct PushAudioResponse {
    pub ok: bool,
    /// True when this push closed an utterance and triggered a turn.
    pub turn_started: bool,
}

/// Inputs to `openhuman.voice_assistant_poll_response`.
#[derive(Debug, Clone, Deserialize)]
pub struct PollResponseRequest {
    pub session_id: String,
}

/// Outputs from `openhuman.voice_assistant_poll_response`.
#[derive(Debug, Clone, Serialize)]
pub struct PollResponseResponse {
    pub ok: bool,
    /// Base64 PCM16LE since the last poll. Empty when nothing is queued.
    pub pcm_base64: String,
    /// The text transcript of what the user said (populated after STT).
    pub transcript: String,
    /// The assistant's reply text (populated after LLM).
    pub reply_text: String,
    /// True when the current outbound utterance is complete.
    pub utterance_done: bool,
}

/// Inputs to `openhuman.voice_assistant_stop_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct StopSessionRequest {
    pub session_id: String,
}

/// Outputs from `openhuman.voice_assistant_stop_session`.
#[derive(Debug, Clone, Serialize)]
pub struct StopSessionResponse {
    pub ok: bool,
    pub session_id: String,
    pub total_turns: u32,
    pub listened_seconds: f64,
    pub spoken_seconds: f64,
}

/// Inputs to `openhuman.voice_assistant_get_status`.
#[derive(Debug, Clone, Deserialize)]
pub struct GetStatusRequest {
    pub session_id: String,
}

/// Outputs from `openhuman.voice_assistant_get_status`.
#[derive(Debug, Clone, Serialize)]
pub struct GetStatusResponse {
    pub ok: bool,
    pub session_id: String,
    pub state: SessionState,
    pub total_turns: u32,
    pub stt_provider: String,
    pub tts_provider: String,
    pub last_error: Option<String>,
}

/// Voice assistant session state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Listening for speech input.
    Listening,
    /// Processing (STT → LLM → TTS pipeline running).
    Processing,
    /// Speaking the response.
    Speaking,
    /// Session stopped.
    Stopped,
    /// Wake word listening (low-power, waiting for activation phrase).
    WakeWordListening,
}

/// Inputs to `openhuman.voice_assistant_interrupt`.
#[derive(Debug, Clone, Deserialize)]
pub struct InterruptRequest {
    pub session_id: String,
}

/// Outputs from `openhuman.voice_assistant_interrupt`.
#[derive(Debug, Clone, Serialize)]
pub struct InterruptResponse {
    pub ok: bool,
    pub was_speaking: bool,
    pub discarded_samples: usize,
}
