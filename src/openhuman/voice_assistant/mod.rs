//! Voice assistant domain — standalone local-first voice interaction.
//!
//! Provides a conversational voice assistant session that uses local STT
//! (whisper.cpp) and local TTS (Piper) by default, with cloud fallback.
//! The session loop is: mic → VAD → STT → LLM → TTS → speaker.
//!
//! Exposed through the controller registry under the `voice_assistant`
//! namespace with six RPC endpoints:
//!
//! - `voice_assistant.start_session`
//! - `voice_assistant.push_audio`
//! - `voice_assistant.poll_response`
//! - `voice_assistant.get_status`
//! - `voice_assistant.interrupt`
//! - `voice_assistant.stop_session`
//!
//! Also provides WebSocket streaming transport at `/ws/voice/{session_id}`.
//!
//! ## Architecture
//!
//! Reuses existing infrastructure:
//! - `voice::factory` for STT/TTS provider dispatch
//! - `meet_agent::ops::Vad` for voice activity detection
//! - `meet_agent::wav` for PCM → WAV packing
//! - `inference::provider::reliable` for LLM chat completions
//!
//! ## Features
//!
//! - Barge-in / interruption handling (auto-detects speech during TTS playback)
//! - Streaming STT with partial transcripts (LocalAgreement chunked approach)
//! - Multi-language detection and auto-switching (Unicode script + diacritics)
//! - Emotion/sentiment detection (keyword heuristics, LLM-based in future)
//! - Wake word detection (energy gate + fuzzy STT keyword matching)
//! - WebSocket binary streaming (eliminates polling overhead)
//!
//! ## Log prefix
//!
//! `[voice-assistant-*]` — brain, session, rpc, ws sub-prefixes.

mod brain;
pub mod noise_cancel;
mod rpc;
mod schemas;
mod session;
mod types;
pub mod wake_word;
pub mod ws_transport;

pub use schemas::{
    all_controller_schemas as all_voice_assistant_controller_schemas,
    all_registered_controllers as all_voice_assistant_registered_controllers,
    schemas as voice_assistant_schemas,
};
pub use types::{SessionState, StartSessionRequest, StopSessionRequest};
