//! Google Meet integration domain.
//!
//! Lets a user ask the agent to join a Google Meet call as an anonymous
//! guest. The core's responsibility is narrow:
//!
//!  - Validate that the supplied URL is a Google Meet meeting URL.
//!  - Validate / trim the guest display name.
//!  - Mint a `request_id` the desktop shell uses to label the per-call
//!    webview window and its data directory.
//!
//! Everything to do with actually opening a CEF webview, driving Meet's
//! join page over CDP, or surfacing a virtual camera lives in the Tauri
//! shell (`app/src-tauri/src/...`) — keeping platform-specific code out
//! of the core.
//!
//! ## Module layout
//!
//! - [`types`]   — request/response types for the join RPC
//! - [`ops`]     — pure validation helpers (URL + display-name)
//! - [`rpc`]     — async JSON-RPC handler functions
//! - [`schemas`] — controller schema definitions and registered handler wrappers
//! - [`agent`]       — the live in-call STT/LLM/TTS loop (was `meet_agent`)
//! - [`backend_bot`] — the backend-delegated Meet bot (was `agent_meetings`)
//!
//! ## Gating
//!
//! This module is the `meet` family **facade**: `pub mod meet;` in
//! `src/openhuman/mod.rs` is always compiled, and every submodule below carries
//! its own `#[cfg(feature = "meet")]`. The parent has to stay ungated because
//! [`backend_bot`] is a facade+stub domain — three always-compiled callers (the
//! heartbeat planner and two subscriber registrations) resolve its stub in a
//! `meet`-less build. The set of items that compiles in each configuration is
//! unchanged from when `meet`/`meet_agent`/`agent_meetings` were three
//! top-level modules; only their paths moved.

#[cfg(feature = "meet")]
pub mod ops;
#[cfg(feature = "meet")]
pub mod rpc;
#[cfg(feature = "meet")]
pub mod schemas;
#[cfg(feature = "meet")]
pub mod types;

// Both carry their own per-submodule gating (and, for `backend_bot`, a
// `#[cfg(not(feature = "meet"))]` stub), so they are declared unconditionally.
pub mod agent;
pub mod backend_bot;

#[cfg(feature = "meet")]
pub use schemas::{
    all_controller_schemas as all_meet_controller_schemas,
    all_registered_controllers as all_meet_registered_controllers,
};
#[cfg(feature = "meet")]
pub use types::*;
