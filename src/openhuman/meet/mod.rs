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

pub mod ops;
pub mod rpc;
pub mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_meet_controller_schemas,
    all_registered_controllers as all_meet_registered_controllers,
};
pub use types::*;
