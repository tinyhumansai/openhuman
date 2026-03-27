//! QuickJS skill runtime module.
//!
//! Provides a persistent JavaScript execution environment for skills
//! using the QuickJS engine via `rquickjs`.
//!
//! Note: The skill runtime is only available on desktop platforms.
//! On mobile (Android/iOS), the skill runtime is disabled.

// Platform-agnostic modules
pub mod loader;
pub mod manifest;
pub mod preferences;
pub mod socket_manager;
pub mod types;
pub mod utils;

// QuickJS modules - desktop only (not available on Android/iOS)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod bridge;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod cron_scheduler;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod ping_scheduler;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod registry;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod skill_registry;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod qjs_engine;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod qjs_skill_instance;
