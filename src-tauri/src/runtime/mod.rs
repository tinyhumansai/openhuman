//! QuickJS skill runtime module.
//!
//! Provides a persistent JavaScript execution environment for skills
//! using the QuickJS engine via `rquickjs`.
//!
//! Note: The skill runtime is desktop-only in this host.

// Runtime implementation now lives fully in rust-core.
pub use openhuman_core::runtime::types;

pub use openhuman_core::runtime::{qjs_engine, socket_manager};
