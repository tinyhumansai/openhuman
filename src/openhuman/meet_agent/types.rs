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
    /// Display name of the call owner — the human who launched the
    /// bot. Used by the wake-word gate in [`crate::openhuman::meet_agent::session`]
    /// as the *only* speaker allowed to issue tool calls. Captions
    /// from any other participant are dropped without recording an
    /// event. Empty string fails closed (no wake fires) so a
    /// misconfigured shell can never expose the user's tool surface.
    /// Defaulted so older shells / smoke tests that don't yet set
    /// the field still parse the payload.
    #[serde(default)]
    pub owner_display_name: String,
    /// Display name the bot uses as its Meet participant tile.
    /// Captions whose `speaker` matches this name are treated as the
    /// bot's own TTS echoing back and dropped — without an explicit
    /// filter the bot would re-wake on its own voice. Empty disables
    /// the filter; dedup + cooldown still apply but it's a weaker
    /// posture.
    #[serde(default)]
    pub bot_display_name: String,
    /// Normalised Meet URL the call joined. Persisted into the
    /// recent-calls log so the UI can show "Joined `…/abc-defg-hij`
    /// — 12 min ago". Defaulted so older shells that haven't been
    /// updated to forward the URL still parse the payload.
    #[serde(default)]
    pub meet_url: String,
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

/// Inputs to `openhuman.meet_agent_push_caption`.
///
/// One row per new line scraped from Meet's captions DOM. Sent by the
/// shell's `caption_listener` every ~500 ms. The wake-word state
/// machine in the brain (see `brain::on_caption`) decides whether to
/// fire a turn.
#[derive(Debug, Clone, Deserialize)]
pub struct PushCaptionRequest {
    pub request_id: String,
    /// Speaker label scraped from Meet (the participant's display
    /// name); empty when the captions row didn't expose one.
    #[serde(default)]
    pub speaker: String,
    /// Caption transcript. Already trimmed by the page-side bridge.
    pub text: String,
    /// `Date.now()` from the page when the line was queued. Used
    /// only for ordering / staleness — the brain treats it as opaque.
    #[serde(default)]
    pub ts_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushCaptionResponse {
    pub ok: bool,
    /// True when this caption tripped the wake-word and a brain turn
    /// is now in flight.
    pub turn_started: bool,
}

/// Inputs to `openhuman.meet_agent_list_calls`.
///
/// Returns the most recently completed Meet calls (newest first) so
/// the Skills "Meeting Bots" card can render a history list inside
/// the same modal the user used to launch the call. Capped server-
/// side at `store::MAX_RECENT_CALLS` so a misconfigured client
/// can't request an unbounded read.
#[derive(Debug, Clone, Deserialize)]
pub struct ListCallsRequest {
    /// Maximum rows to return. Defaults to 50 if absent. Hard cap
    /// applied server-side regardless of what the caller asks for.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Outputs from `openhuman.meet_agent_list_calls`.
#[derive(Debug, Clone, Serialize)]
pub struct ListCallsResponse {
    pub ok: bool,
    pub calls: Vec<super::store::MeetCallRecord>,
    /// Number of rows in `calls`. Convenient for the UI when
    /// rendering a header like "Recent calls (12)".
    pub count: usize,
}

/// Inputs to `openhuman.meet_agent_get_call_detail`.
///
/// Loads the transcript + generated summary for a single completed call so the
/// recent-calls panel can expand a row without bloating the list payload.
#[derive(Debug, Clone, Deserialize)]
pub struct GetCallDetailRequest {
    /// request_id of the call. Matches the `request_id` field of the
    /// `MeetCallRecord` rows returned by `list_calls`.
    pub request_id: String,
}

/// Outputs from `openhuman.meet_agent_get_call_detail`.
#[derive(Debug, Clone, Serialize)]
pub struct GetCallDetailResponse {
    pub ok: bool,
    /// The persisted detail, or `null` when none exists for this call (older
    /// calls recorded before the feature, or a best-effort detail write that
    /// failed). The UI degrades to "no transcript yet" in that case.
    pub detail: Option<super::store::MeetCallDetail>,
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
