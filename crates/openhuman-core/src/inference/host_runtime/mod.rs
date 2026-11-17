//! OpenHuman policy, RPC, and speech bindings for `tinyinference-local`.
//!
//! Endpoint resolution, probing, and inference execution live in
//! TinyInference. The local runtime itself is run by the user; nothing here
//! downloads models or manages a runtime process. This module retains only product-owned access
//! policy, controller wiring, and voice integration.

#[cfg(test)]
pub(crate) static INFERENCE_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Blocking form for synchronous tests (panics inside a tokio runtime).
#[cfg(test)]
pub(crate) fn inference_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    INFERENCE_TEST_MUTEX.blocking_lock()
}

/// Async form for `#[tokio::test]` bodies, so the guard may be held across `.await`.
#[cfg(test)]
pub(crate) async fn inference_test_guard_async() -> tokio::sync::MutexGuard<'static, ()> {
    INFERENCE_TEST_MUTEX.lock().await
}

mod core;
pub mod ops;
mod schemas;

// `pub(crate)` so the shared `apply_no_window` helper can be reused from the
// agent shell runtime (`modules::tinybox::host_runtime`) — single source of truth for the
// Windows `CREATE_NO_WINDOW` flag (#3727/#3728).
pub mod service;

pub use core::*;

pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_local_inference_controller_schemas,
    all_registered_controllers as all_local_inference_registered_controllers,
};
