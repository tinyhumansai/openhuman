//! Realtime voice-agent turn handler (#5399).
//!
//! The backend relays each turn of an ElevenLabs Agents session down the socket
//! as `voice:harness { correlationId, messages }` (see the backend's
//! voice-agent Custom-LLM relay). We run the **local
//! orchestrator agent** — the same brain the chat UI and meet bot use, with the
//! user's tools/memory/MCP — and stream the reply back up as
//! `voice:harness:delta` / `voice:harness:done` (or `:error`). This is what
//! keeps a cloud realtime voice session backed by the desktop-local brain.
//!
//! Approval-gate origin: **ExternalChannel** — the turn text is user speech
//! arriving over a channel, so `external_effect` tools route through the
//! audit-trail path rather than running with trusted-CLI semantics.
//!
//! ## Module layout
//!
//! - [`prompt`] — pure prompt extraction/classification helpers.
//! - [`agent`] — building and running the per-turn voice orchestrator.
//! - [`chat_delivery`] — delivering a deferred result/failure into chat, and
//!   the raw socket emit helpers.
//! - [`turn_handler`] — the end-to-end turn handler that ties them together.

mod agent;
mod chat_delivery;
mod prompt;
mod turn_handler;

pub use prompt::extract_prompt;
pub use turn_handler::handle_voice_harness_turn;

#[cfg(test)]
use prompt::{
    is_answerable_prompt, is_content_free, messages_to_history_pairs, readback_payload,
    should_arm_speak_back, spoken_delta, VOICE_READBACK_PREFIX,
};
#[cfg(test)]
use tinyagents_harness::progress::AgentProgress;
#[cfg(test)]
use turn_handler::{next_handoff_line, VOICE_HANDOFF_LINES};

#[cfg(test)]
#[path = "realtime_harness_tests.rs"]
mod tests;
