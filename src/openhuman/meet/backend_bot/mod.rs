//! Agent Meetings integration domain.
//!
//! Delegates Google Meet bot joining/leaving to the TinyHumans backend
//! via the existing Socket.IO connection (`SocketManager`). The backend
//! runs a Camoufox headless browser that joins the meeting, captures
//! captions, and streams LLM decisions back over Socket.IO events
//! (`bot:reply`, `bot:harness`, `bot:transcript`).
//!
//! ## Module layout
//!
//! - [`types`]   — request/response types + meeting session model
//! - [`ops`]     — RPC handlers that emit Socket.IO events
//! - [`schemas`] — controller schema + registered handler wrappers
//! - [`store`]   — SQLite persistence for meeting sessions
//! - [`in_call`] — Phase 2 in-call agency: wake-phrase command → orchestrator → `bot:speak`
//!
//! ## Compile-time gating (`meet` feature, #4800)
//!
//! All submodules are `#[cfg(feature = "meet")]`. Three always-compiled call
//! sites reach in here for non-registration symbols — the heartbeat planner
//! (`calendar::handle_calendar_meeting_candidate`) plus two subscriber
//! registrations (`core::jsonrpc`, `channels::runtime::startup`) — so a
//! `#[cfg(not(feature = "meet"))]` [`stub`] supplies no-op equivalents and
//! those callers need no cfg of their own.

#[cfg(feature = "meet")]
pub mod bus;
#[cfg(feature = "meet")]
pub mod calendar;
#[cfg(feature = "meet")]
pub mod in_call;
#[cfg(feature = "meet")]
pub mod ops;
#[cfg(feature = "meet")]
pub mod recent_calls;
#[cfg(feature = "meet")]
pub mod schemas;
#[cfg(feature = "meet")]
pub mod store;
#[cfg(feature = "meet")]
pub mod summary;
#[cfg(feature = "meet")]
pub mod types;
#[cfg(feature = "meet")]
pub mod upcoming;

#[cfg(not(feature = "meet"))]
mod stub;
#[cfg(not(feature = "meet"))]
pub use stub::*;

#[cfg(feature = "meet")]
pub use schemas::{
    all_controller_schemas as all_agent_meetings_controller_schemas,
    all_registered_controllers as all_agent_meetings_registered_controllers,
};
