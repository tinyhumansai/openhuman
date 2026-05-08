//! Request / response types for the `meet_agent` domain.
//!
//! Audio frames cross the RPC boundary as base64-encoded PCM16LE @ 16kHz
//! mono. Base64 (rather than raw bytes) because JSON-RPC transports the
//! envelope as JSON and binary bytes don't survive the trip — the shell
//! decodes/encodes at the `core_rpc` boundary, mirroring how the existing
//! `voice::streaming` WebSocket path moves audio.

use serde::{Deserialize, Serialize};

/// Inputs to `openhuman.meet_agent_start_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct StartSessionRequest {
    /// `request_id` minted by `openhuman.meet_join_call`. Used as the
    /// session key so the shell's existing per-call book-keeping (window
    /// label, data dir) lines up with the agent loop's session.
    pub request_id: String,
    /// Sample rate of the PCM frames the shell will push. Must match
    /// what `voice::streaming` expects (16000) — the shell is responsible
    /// for resampling the CEF audio handler's native rate down before
    /// sending. Validated on entry.
    #[serde(default = "default_sample_rate")]
    pub sample_rate_hz: u32,
}

fn default_sample_rate() -> u32 {
    16_000
}

/// Outputs from `openhuman.meet_agent_start_session`.
#[derive(Debug, Clone, Serialize)]
pub struct StartSessionResponse {
    pub ok: bool,
    pub request_id: String,
    /// Echoed sample rate the session was opened with — the shell pins
    /// its resampler to this.
    pub sample_rate_hz: u32,
}

/// Inputs to `openhuman.meet_agent_push_listen_pcm`.
///
/// Sent every ~100ms while the call is open. Small frames keep VAD
/// responsive without overloading the JSON envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct PushListenPcmRequest {
    pub request_id: String,
    /// Base64-encoded PCM16LE samples at the session's `sample_rate_hz`.
    /// Empty string is allowed and treated as "no audio this tick"
    /// (used by the shell to keep the keep-alive heartbeat without a
    /// payload when CEF reports silence).
    pub pcm_base64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushListenPcmResponse {
    pub ok: bool,
    /// True when this push triggered a VAD-detected end-of-utterance and
    /// the brain ran a turn. The shell can use this as a UI hint
    /// ("agent is thinking…").
    pub turn_started: bool,
}

/// Inputs to `openhuman.meet_agent_poll_speech`.
///
/// Pull-style: the shell calls this periodically and gets any PCM the
/// brain has synthesized since the last poll. Pull beats push here
/// because the shell is the side that knows whether the virtual mic is
/// actually draining (back-pressure lives there, not in core).
#[derive(Debug, Clone, Deserialize)]
pub struct PollSpeechRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PollSpeechResponse {
    pub ok: bool,
    /// Base64-encoded PCM16LE @ session sample rate, or empty when there
    /// is nothing queued. The shell appends this to its UDS feed.
    pub pcm_base64: String,
    /// True when the brain has finished synthesizing the current
    /// utterance and the shell can flush + drop back to silence.
    pub utterance_done: bool,
}

/// Inputs to `openhuman.meet_agent_stop_session`.
#[derive(Debug, Clone, Deserialize)]
pub struct StopSessionRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopSessionResponse {
    pub ok: bool,
    pub request_id: String,
    /// Total seconds of inbound audio the session processed — useful
    /// for telemetry and the smoke test in [`crate::openhuman::meet_agent`].
    pub listened_seconds: f32,
    /// Total seconds of outbound audio the session synthesized.
    pub spoken_seconds: f32,
    /// Number of completed agent turns (one transcript + one TTS reply).
    pub turn_count: u32,
}

/// Lightweight transcript / event record kept per session. Exposed so
/// the shell can render a live captions overlay and so the json_rpc_e2e
/// test can assert turn boundaries.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    pub kind: SessionEventKind,
    pub text: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    /// Final STT transcript for an inbound utterance.
    Heard,
    /// Outbound text the agent decided to speak.
    Spoke,
    /// Internal note (errors, "agent declined to respond", etc).
    Note,
}
