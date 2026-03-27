//! QuickJS skill runtime module.
//!
//! Provides a persistent JavaScript execution environment for skills
//! using the QuickJS engine via `rquickjs`.
//!
//! Note: The skill runtime is only available on desktop platforms.
//! On mobile (Android/iOS), the skill runtime is disabled.

// Runtime implementation now lives fully in rust-core.
pub use rust_core::runtime::{loader, manifest, preferences, types, utils};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use rust_core::runtime::{
    bridge, cron_scheduler, ping_scheduler, qjs_engine, qjs_skill_instance, skill_registry,
    socket_manager,
};
