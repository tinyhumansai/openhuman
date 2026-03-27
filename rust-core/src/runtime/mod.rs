pub mod loader;
pub mod manifest;
pub mod preferences;
pub mod types;
pub mod utils;

#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod bridge;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod cron_scheduler;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod ping_scheduler;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod qjs_engine;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod qjs_skill_instance;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod quickjs_libs;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod skill_registry;
#[cfg(all(feature = "tauri-host", not(any(target_os = "android", target_os = "ios"))))]
pub mod socket_manager;
