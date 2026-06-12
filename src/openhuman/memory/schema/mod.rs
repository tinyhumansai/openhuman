//! Controller schemas for the memory tree.
//!
//! Registered JSON-RPC methods include the original Phase 1 surface
//! (`ingest`, `list_chunks`, `get_chunk`) plus the new
//! Memory-tab read RPCs added by the cloud-default backend refactor:
//! `list_sources`, `search`, `recall`, `entity_index_for`,
//! `top_entities`, `chunk_score`, `delete_chunk`, and destructive
//! maintenance helpers for local iteration.
//!
//! Handlers delegate to [`super::rpc`] (write side) or
//! [`super::read_rpc`] (UI read side).
//!
//! # Sub-module layout
//!
//! | File              | Contents                                             |
//! |-------------------|------------------------------------------------------|
//! | `definitions.rs`  | [`schemas`] match — one [`ControllerSchema`] per RPC |
//! | `handlers.rs`     | `handle_*` functions bridging JSON → typed RPC calls |
//! | `registry.rs`     | [`all_controller_schemas`] / [`all_registered_controllers`] lists |

mod definitions;
mod handlers;
mod registry;

pub use definitions::schemas;
pub use registry::{all_controller_schemas, all_registered_controllers};

// Re-export the NAMESPACE constant so schema_tests.rs can reference it via
// `super::NAMESPACE` the same way the original flat module did.
pub(crate) use definitions::NAMESPACE;

#[cfg(test)]
#[path = "../schema_tests.rs"]
mod tests;
