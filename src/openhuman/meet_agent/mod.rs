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
//!
//! ## Compile-time gating (`meet` feature, #4800)
//!
//! Every submodule here is `#[cfg(feature = "meet")]` — **except [`wav`]**.
//!
//! ### ⚠ The `wav` carve-out is load-bearing — do not "tidy" it
//!
//! [`wav::pack_pcm16le_mono_wav`] is called by
//! `desktop_companion::pipeline::stt`, which is `DomainGroup::Platform` and is
//! therefore compiled in **every** build, including `--no-default-features`.
//! `wav` must stay ungated so that call site keeps its **real** implementation.
//!
//! This is safe to leave ungated at zero cost: `wav.rs` is a self-contained
//! hand-rolled RIFF writer with no `use` statements and no dependencies. It
//! pulls in nothing when the rest of the domain is compiled out. (It is
//! hand-rolled precisely so Meet never needed `hound`, which the `voice` gate
//! already owns and sheds.)
//!
//! **If you ever add `#[cfg(feature = "meet")]` to `pub mod wav;`**, the
//! `--no-default-features` build will fail loudly at `desktop_companion`. That
//! failure is *correct and useful*. Do **not** "fix" it by stubbing
//! `pack_pcm16le_mono_wav` to return an empty/placeholder buffer: that turns a
//! compile error into green CI while silently corrupting desktop-companion STT
//! forever (the STT backend would receive a malformed WAV). Revert the cfg
//! instead — or, if `wav` genuinely must move, relocate it to a always-compiled
//! home rather than stubbing it.

#[cfg(feature = "meet")]
pub mod brain;
#[cfg(feature = "meet")]
pub mod ops;
#[cfg(feature = "meet")]
pub mod rpc;
#[cfg(feature = "meet")]
pub mod schemas;
#[cfg(feature = "meet")]
pub mod session;
#[cfg(feature = "meet")]
pub mod store;
#[cfg(feature = "meet")]
pub mod types;
// NOT gated — see the carve-out note above. `desktop_companion` (always-on)
// depends on the real implementation.
pub mod wav;

#[cfg(feature = "meet")]
pub use schemas::{
    all_controller_schemas as all_meet_agent_controller_schemas,
    all_registered_controllers as all_meet_agent_registered_controllers,
};
#[cfg(feature = "meet")]
pub use session::{MeetAgentSession, MeetAgentSessionRegistry, SESSION_REGISTRY};
#[cfg(feature = "meet")]
pub use types::*;
