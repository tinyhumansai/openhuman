//! Meet-agent domain — listening + speaking loop for a live Google Meet
//! call.
//!
//! Sits *next to* `meet/` (which only validates a URL and mints a
//! `request_id`) and reuses `voice/` for STT/TTS. Where `meet/` is
//! single-shot ("here is a request_id, shell goes off and opens a
//! window"), `meet_agent/` is a long-lived session: while the call is
//! open, the Tauri shell streams PCM frames from the CEF audio handler
//! into the core; the core runs VAD-segmented STT, decides whether to
//! reply, runs TTS, and streams synthesized PCM back out to the shell's
//! virtual-mic pump.
//!
//! ## Why a separate domain (not just more functions on `meet/`)?
//!
//! `meet/` is intentionally pure-validation — no state, no streams, no
//! audio. A live agentic loop is the opposite shape: a session registry,
//! per-session ring buffers, VAD/turn state, transcript log, and a TTS
//! pipeline. Bolting that onto `meet/` would force the validation surface
//! to drag in audio dependencies. Splitting keeps each domain small.
//!
//! ## Module layout
//!
//! - [`types`]    — request/response types, public session events
//! - [`ops`]      — VAD, ring-buffer, transcript helpers (pure, testable)
//! - [`session`]  — `MeetAgentSession` and the per-session registry
//! - [`brain`]    — turn orchestration: STT → LLM → TTS (stub in PR1)
//! - [`rpc`]      — JSON-RPC handlers
//! - [`schemas`]  — controller schema definitions
//!
//! ## RPC surface
//!
//! - `openhuman.meet_agent_start_session`  — open a session for a `request_id`
//! - `openhuman.meet_agent_push_listen_pcm` — shell pushes captured PCM frames
//! - `openhuman.meet_agent_poll_speech`     — shell pulls synthesized PCM frames
//! - `openhuman.meet_agent_stop_session`    — close session, flush pending audio

pub mod brain;
pub mod ops;
pub mod rpc;
pub mod schemas;
pub mod session;
pub mod types;
pub mod wav;

pub use schemas::{
    all_controller_schemas as all_meet_agent_controller_schemas,
    all_registered_controllers as all_meet_agent_registered_controllers,
};
pub use session::{MeetAgentSession, MeetAgentSessionRegistry, SESSION_REGISTRY};
pub use types::*;
