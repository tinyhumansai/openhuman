//! JS-to-Rust bridge modules for the skill runtime.
//!
//! Currently only `net` is actively used (by V8 ops).
//! Other modules are preserved for potential future use.

#[allow(dead_code)]
pub mod cron_bridge;
#[allow(dead_code)]
pub mod db;
#[allow(dead_code)]
pub mod log_bridge;
pub mod net;
#[allow(dead_code)]
pub mod skills_bridge;
#[allow(dead_code)]
pub mod store;
#[allow(dead_code)]
pub mod tauri_bridge;
