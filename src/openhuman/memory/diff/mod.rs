//! Host layer over [`tinymemory_core::diff`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::diff::…` path resolving.

pub use tinymemory_core::diff::*;

#[cfg(feature = "memory-git")]
pub mod rpc;
#[cfg(feature = "memory-git")]
pub mod schemas;

#[cfg(feature = "memory-git")]
pub use schemas::{
    all_controller_schemas as all_memory_diff_controller_schemas,
    all_registered_controllers as all_memory_diff_registered_controllers,
};
// The agent tool came back with the rest of them; re-exported here so the
// historical `memory::diff::MemoryDiffTool` path keeps resolving.
#[cfg(feature = "memory-git")]
pub use crate::openhuman::memory::tools::diff::MemoryDiffTool;
#[cfg(not(feature = "memory-git"))]
mod stub;

#[cfg(not(feature = "memory-git"))]
pub use stub::{all_memory_diff_controller_schemas, all_memory_diff_registered_controllers};
