//! Conversation thread and message management.
//!
//! Thread lifecycle (create, list, delete, purge) and per-thread message
//! CRUD. Storage is [`store`] (JSONL files over `tinyagents_session::threads`);
//! this module owns the RPC surface and controller registry.

pub mod error;
pub mod ops;
pub mod rpc_models;
pub mod schemas;
pub mod store;
#[cfg(test)]
mod transcript_host_tests;
pub mod turn_state;
pub mod welcome_migration;

pub use error::{ThreadsError, THREAD_NOT_FOUND_KIND};
pub use rpc_models::*;
pub use schemas::{
    all_controller_schemas as all_threads_controller_schemas,
    all_registered_controllers as all_threads_registered_controllers,
};
pub use welcome_migration::{migrate_welcome_agent_artifacts, WelcomeMigrationResult};

/// Log prefix for thread-title generation (grep-friendly).
pub(crate) const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
