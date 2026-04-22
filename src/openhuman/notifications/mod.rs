//! Integration notification domain.
//!
//! Captures notifications from embedded webview integrations (WhatsApp Web,
//! Gmail, Slack, …), runs them through the triage LLM pipeline, and stores
//! them in a unified notification center accessible via the RPC surface.
//!
//! ## Module layout
//!
//! - [`types`]   — `IntegrationNotification`, `NotificationStatus`, request/response types
//! - [`store`]   — SQLite persistence (one DB per workspace)
//! - [`rpc`]     — Async RPC handler functions: ingest, list, mark_read
//! - [`schemas`] — Controller schema definitions and registered handler wrappers

pub mod rpc;
pub mod schemas;
pub mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_notifications_controller_schemas,
    all_registered_controllers as all_notifications_registered_controllers,
};
pub use types::*;
