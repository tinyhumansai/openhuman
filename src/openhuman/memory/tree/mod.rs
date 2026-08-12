//! Host layer over [`tinymemory_core::tree`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::tree::…` path resolving.

pub use tinymemory_core::tree::*;

pub mod health;
pub mod retrieval;
// `tree::tree` mirrors `tinymemory_core::tree::tree` — the wrapper has to keep
// the extracted crate's path shape so every historical `memory::tree::tree::…`
// reference still resolves. Renaming it here would break that for a lint.
#[allow(clippy::module_inception)]
pub mod tree;
pub mod tree_runtime;

// Controller registries. These aggregate the RPC surface that stayed here, so
// they cannot live in the extracted crate alongside the rest of `tree`.
pub use crate::openhuman::memory::schema::{
    all_controller_schemas as all_memory_tree_controller_schemas,
    all_registered_controllers as all_memory_tree_registered_controllers,
};
pub use retrieval::{all_retrieval_controller_schemas, all_retrieval_registered_controllers};
pub use tree_runtime::{
    all_tree_summarizer_controller_schemas, all_tree_summarizer_registered_controllers,
};
