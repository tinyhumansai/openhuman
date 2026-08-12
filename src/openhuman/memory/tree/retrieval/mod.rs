//! Host layer over [`tinymemory_core::tree::retrieval`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::tree::retrieval::…` path resolving.

pub use tinymemory_core::tree::retrieval::*;

pub mod rpc;
pub mod schemas;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_retrieval_controller_schemas,
    all_registered_controllers as all_retrieval_registered_controllers,
};
