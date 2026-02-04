//! V8 skill runtime module.
//!
//! Provides a persistent JavaScript execution environment for skills
//! using the V8 engine via `deno_core`.
//!
//! Note: V8/deno_core is only available on desktop platforms.
//! On mobile (Android/iOS), the skill runtime is disabled.

// Platform-agnostic modules
pub mod loader;
pub mod manifest;
pub mod preferences;
pub mod socket_manager;
pub mod types;

// V8/deno_core modules - desktop only (no prebuilt binaries for Android/iOS)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod bridge;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod cron_scheduler;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod skill_registry;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod v8_engine;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod v8_skill_instance;
