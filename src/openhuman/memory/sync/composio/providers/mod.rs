//! Host layer over [`tinymemory_core::sync::composio::providers`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::sync::composio::providers::…` path resolving.

pub use tinymemory_core::sync::composio::providers::*;

pub mod slack;

/// Backend-tenant client access for legacy provider helpers — see the module.
pub mod context_ext;

pub use context_ext::ProviderContextExt;
