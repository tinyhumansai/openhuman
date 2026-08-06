//! Turn orchestration: STT → LLM → TTS.
//!
//! ## Pipeline
//!
//! When [`session::Vad`] reports `EndOfUtterance`, [`run_turn`] drains
//! the inbound buffer and runs three serial stages:
//!
//! 1. **STT** — wrap the PCM16LE samples in a WAV container and post
//!    to [`crate::openhuman::voice::cloud_transcribe`]. Returns the
//!    transcribed text (or `Err` on transport / auth failure).
//!
//! 2. **LLM** — send a tiny chat-completions request through
//!    [`crate::api::BackendOAuthClient`] with a "live meeting agent"
//!    system prompt and the transcript as the user message. Returns a
//!    short reply (or empty string when the agent decides to stay
//!    silent).
//!
//! 3. **TTS** — feed the reply text into
//!    [`crate::openhuman::voice::reply_speech`] requesting
//!    `output_format = "pcm_16000"`. Decode the base64 PCM bytes back
//!    into `Vec<i16>` and enqueue on the session's outbound queue.
//!
//! ## Fallback
//!
//! When the backend session token is missing (the most common reason
//! a stage fails outside production: tests, no-network smoke runs),
//! we fall back to deterministic stubs so the loop still produces an
//! audible blip and the unit tests stay network-free. Real
//! transport / 5xx errors are *not* swallowed — they surface as
//! `Note` events so a real-call failure is visible in the transcript
//! log, not silently degraded to a stub.

mod access;
mod constants;
mod llm;
mod speech;
mod stubs;
mod text;
mod turns;

// ─── Public API (unchanged external surface) ────────────────────────

pub use access::{run_grant_turn, run_soft_deny_turn};
pub use turns::{run_caption_turn, run_turn};

// Speech sanitizer shared with `agent_meetings::in_call` — both paths
// feed orchestrator replies into TTS and need the same markdown /
// reasoning-trace stripping.
pub(crate) use text::strip_for_speech;

use constants::agent_cache;

/// Drop the cached orchestrator for a meet session. Called from
/// `handle_stop_session` so a finished call doesn't leak the Agent
/// (each one carries memory tree + tool registry handles).
pub async fn forget_session_agent(request_id: &str) {
    let mut guard = agent_cache().lock().await;
    if guard.remove(request_id).is_some() {
        log::info!("[meet-agent] dropped cached orchestrator for request_id={request_id}");
    }
}

// ─── Test surface (items accessed by brain_tests.rs) ────────────────
// brain_tests.rs uses `super::*` and accesses private items directly,
// so we expose what the tests need via a `#[cfg(test)]` re-export block.

#[cfg(test)]
pub(crate) use access::{
    classify_unauthorized_intent, looks_like_grant_intent, soft_deny_message, UnauthorizedIntent,
};
#[cfg(test)]
pub(crate) use llm::extract_chat_completion_text;
#[cfg(test)]
pub(crate) use text::recent_dialog_history;

#[cfg(test)]
#[path = "../brain_tests.rs"]
mod tests;
