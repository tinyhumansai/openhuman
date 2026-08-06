//! AES-256-GCM encryption layer for AI memory storage.
//!
//! All memory data (SQLite content, embeddings, session transcripts) is
//! encrypted at rest using AES-256-GCM. Keys are derived from a user
//! password via Argon2id.

mod core;
pub mod ops;
mod schemas;

pub use core::*;
pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_encryption_controller_schemas,
    all_registered_controllers as all_encryption_registered_controllers,
};
