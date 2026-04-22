//! Native Gmail integration domain.
//!
//! Provides browser-driven Gmail data ingestion via the Tauri-side
//! `gmail_scanner` (CDP network MITM + IndexedDB backfill + DOM signals).
//! The core domain owns account lifecycle, memory ingestion, and cron
//! scheduling. The scanner runs in the Tauri shell and forwards messages
//! here via `ingest_messages`.
//!
//! Exposed through the controller registry as `openhuman.gmail_*` RPC methods.

pub mod bus;
pub mod ingest;
pub mod ops;
mod rpc;
mod schemas;
pub mod store;
pub mod types;

// Re-export types used by the scanner.
pub use types::{GmailAccount, GmailMessage, GmailSyncStats};

// Controller registry wiring (consumed by src/core/all.rs).
pub use schemas::{
    all_controller_schemas as all_gmail_controller_schemas,
    all_registered_controllers as all_gmail_registered_controllers,
};
