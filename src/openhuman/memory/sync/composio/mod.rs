//! Host layer over [`tinymemory_core::sync::composio`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//! The glob re-export keeps every historical `memory::sync::composio::…` path resolving.

pub use tinymemory_core::sync::composio::*;

pub mod providers;

pub mod bus;

pub use bus::{
    register_composio_trigger_subscriber, ComposioConfigChangedSubscriber,
    ComposioTriggerSubscriber,
};
