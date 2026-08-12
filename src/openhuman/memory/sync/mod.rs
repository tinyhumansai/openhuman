//! Host layer over [`tinymemory_core::sync`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::sync::…` path resolving.

pub use tinymemory_core::sync::*;

pub mod composio;
pub mod sync_status;
