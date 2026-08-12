//! Host layer over [`tinymemory_core::sources`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::sources::…` path resolving.

pub use tinymemory_core::sources::*;

pub mod rpc;
pub mod schemas;

pub use tinymemory_core::sources::apply_kind_defaults;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_memory_sources_controller_schemas,
    all_registered_controllers as all_memory_sources_registered_controllers,
};
