//! Live voice agents: realtime, two-way speech with the agent, including tool
//! calls.
//!
//! - [`providers`]: the four live providers (Gemini Live via TinyHumans,
//!   ElevenLabs via TinyHumans, Gemini with a Google key, Sarvam) and the
//!   ticket / signed-URL minting the hosted ones need.
//! - [`session`]: starts a session for a chat thread. Tools come from the
//!   orchestrator's session host and run through approvals and tool policy
//!   (`agent::tinyagents::live_harness`) via `tinyagents-live`.
//! - [`ws`]: the `/ws/live-voice` WebSocket that carries audio and events
//!   (`http-server` builds only).
//! - [`persist`]: final transcripts are saved into the thread.
//! - [`ops`] / [`schemas`]: the `voice.live_*` RPCs (catalogue, settings,
//!   provider test).
//!
//! Provider protocols live in `tinyliveagents`; nothing here speaks a
//! provider's wire format.

mod error;
pub mod ops;
mod persist;
mod providers;
pub mod registry;
pub mod schemas;
mod session;
pub mod types;
#[cfg(feature = "http-server")]
pub mod ws;

pub use error::LiveVoiceError;
pub use providers::{PreparedProvider, GEMINI_LANGUAGES, GEMINI_VOICES};
pub use schemas::{live_controller_schemas, live_registered_controllers, live_schemas};
