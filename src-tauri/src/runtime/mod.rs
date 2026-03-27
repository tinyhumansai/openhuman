//! QuickJS skill runtime module.
//!
//! Provides a persistent JavaScript execution environment for skills
//! using the QuickJS engine via `rquickjs`.
//!
//! Note: The skill runtime is desktop-only in this host.

// Runtime implementation now lives fully in rust-core.
pub use openhuman_core::runtime::{loader, manifest, preferences, types, utils};

pub use openhuman_core::runtime::{
    bridge, cron_scheduler, ping_scheduler, qjs_engine, qjs_skill_instance, skill_registry,
    socket_manager,
};
