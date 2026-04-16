//! Conversation thread and message management.
//!
//! Thread lifecycle (create, list, delete, purge) and per-thread message
//! CRUD. Storage delegates to `memory::conversations` JSONL files; this
//! module owns the RPC surface and controller registry.

pub mod ops;
pub mod schemas;

pub use schemas::{
    all_controller_schemas as all_threads_controller_schemas,
    all_registered_controllers as all_threads_registered_controllers,
};
