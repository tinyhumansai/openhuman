//! Host layer over [`tinymemory_core::tree::tree`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::tree::tree::…` path resolving.

pub use tinymemory_core::tree::tree::*;

pub mod rpc;
