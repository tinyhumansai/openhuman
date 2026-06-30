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

pub mod bus;
pub mod calendar;
pub mod in_call;
pub mod ops;
pub mod recent_calls;
pub mod schemas;
pub mod store;
pub mod summary;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_agent_meetings_controller_schemas,
    all_registered_controllers as all_agent_meetings_registered_controllers,
};
