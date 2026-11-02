//! Conversation thread and message management.
//!
//! Thread lifecycle (create, list, delete, purge) and per-thread message
//! CRUD. Storage is [`store`] (the storage backend, through `tinyagents_session::threads`);
//! this module owns the RPC surface and controller registry.

pub mod artifact_rows;
pub(crate) mod attachments;
pub(crate) mod channel_projection;
pub mod error;
pub mod ops;
pub mod rpc_models;
pub(crate) mod run_mode;
pub mod schemas;
pub mod store;
#[cfg(test)]
mod transcript_host_tests;
pub mod turn_state;
pub mod wire_schema;

pub use error::{ThreadsError, THREAD_NOT_FOUND_KIND};
pub use rpc_models::*;
#[doc(hidden)]
pub use run_mode::{parse_mode_label, set_mode};
pub use schemas::{
    all_controller_schemas as all_threads_controller_schemas,
    all_registered_controllers as all_threads_registered_controllers,
};

/// Log prefix for thread-title generation (grep-friendly).
pub(crate) const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
