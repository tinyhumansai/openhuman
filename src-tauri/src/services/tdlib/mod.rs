//! TDLib Integration Module
//!
//! Provides native TDLib access for the Telegram skill.
//! Desktop uses tdlib-rs, Android uses JNI bridge to TDLib Android library.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod manager;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use manager::{TdLibManager, TDLIB_MANAGER};
