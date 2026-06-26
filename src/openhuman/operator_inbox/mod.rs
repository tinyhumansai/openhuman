//! Operator inbox assistant domain.
//!
//! Message triage, priority scoring, draft reply generation, and follow-up scheduling.
//!
//! Log prefix: `[operator_inbox]`

pub mod connection;
pub mod engine;
pub mod imap_client;
pub mod parser;
pub mod poller;
mod rpc;
mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_operator_inbox_controller_schemas,
    all_registered_controllers as all_operator_inbox_registered_controllers,
    schemas as operator_inbox_schemas,
};
pub use types::{MessageSource, TriagePriority, TriageRecord, TriageStatus};
