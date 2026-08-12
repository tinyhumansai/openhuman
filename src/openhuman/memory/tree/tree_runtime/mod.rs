//! Host layer over [`tinymemory_core::tree::tree_runtime`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::tree::tree_runtime::…` path resolving.

pub use tinymemory_core::tree::tree_runtime::*;

pub mod ops;
pub mod schemas;

pub use ops as rpc;

pub mod bus;

/// The `openhuman memory tree` CLI subcommands, which drive the RPC handlers.
pub mod cli;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_tree_summarizer_controller_schemas,
    all_registered_controllers as all_tree_summarizer_registered_controllers,
};
