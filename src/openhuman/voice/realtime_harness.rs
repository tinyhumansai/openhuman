//! Realtime voice-agent turn handler (#5399).
//!
//! The backend relays each turn of an ElevenLabs Agents session down the socket
//! as `voice:harness { correlationId, messages }` (see the backend's
//! `/voice-agent/chat/completions` Custom-LLM relay). We run the **local
//! orchestrator agent** — the same brain the chat UI and meet bot use, with the
//! user's tools/memory/MCP — and stream the reply back up as
//! `voice:harness:delta` / `voice:harness:done` (or `:error`). This is what
//! keeps a cloud realtime voice session backed by the desktop-local brain.
//!
//! Approval-gate origin: **ExternalChannel** — the turn text is user speech
//! arriving over a channel, so `external_effect` tools route through the
//! audit-trail path rather than running with trusted-CLI semantics.

#[cfg(test)]
#[path = "realtime_harness_tests.rs"]
mod tests;
include!("realtime_harness_part_01.rs");
include!("realtime_harness_part_02.rs");
