//! Main-thread bridge for synthetic input (mouse/keyboard).
//!
//! macOS's Text Input Source APIs (`TSMGetInputSourceProperty`), which enigo
//! calls during keyboard-layout lookup, **must run on the app's main thread**.
//! Running them on a tokio worker (or `spawn_blocking`) traps with
//! `_dispatch_assert_queue_fail` / `EXC_BREAKPOINT` and crashes the CEF host
//! (tracker §1.8 / Change 1.15 — confirmed via crash report).
//!
//! So the keyboard/mouse tools never call enigo on their own thread. They build
//! a closure and hand it to [`run_input_on_main`], which dispatches it — over
//! the native request registry — to a handler the Tauri shell registers at
//! startup, which runs it on the real main thread via
//! `AppHandle::run_on_main_thread`.

use crate::core::event_bus::request_native_global;

/// Native-registry method the Tauri shell handles to run an input op on the
/// main thread. The shell registers a handler under this key at startup.
pub const INPUT_ON_MAIN_THREAD_METHOD: &str = "computer.input_on_main_thread";

/// A synthetic-input operation to run on the app's main thread. `run` performs
/// the enigo calls and returns a human-readable success message (`Ok`) or an
/// error string (`Err`). Carried by value through the native registry (no
/// serialization — the boxed `FnOnce` passes through unchanged).
pub struct MainThreadInputOp {
    pub run: Box<dyn FnOnce() -> Result<String, String> + Send>,
}

/// Dispatch `op` to the app main thread and await its result.
///
/// Returns an error when no main-thread executor is registered (headless / CLI
/// builds have no Tauri main thread — synthetic input is a desktop capability).
pub async fn run_input_on_main<F>(op: F) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let req = MainThreadInputOp { run: Box::new(op) };
    match request_native_global::<MainThreadInputOp, Result<String, String>>(
        INPUT_ON_MAIN_THREAD_METHOD,
        req,
    )
    .await
    {
        Ok(inner) => inner,
        Err(e) => Err(format!(
            "synthetic input requires the desktop app's main-thread executor (unavailable: {e})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::register_native_global;

    /// With an executor registered (as the desktop shell does at startup), the
    /// op's result passes straight back through the native registry — both the
    /// success and the error variant. Registering here also exercises
    /// `MainThreadInputOp` construction and the `Ok(inner) => inner` arm.
    #[tokio::test]
    async fn dispatches_op_result_through_registered_executor() {
        // Stand-in for the Tauri main-thread handler: just run the op inline.
        register_native_global::<MainThreadInputOp, Result<String, String>, _, _>(
            INPUT_ON_MAIN_THREAD_METHOD,
            |req| async move { Ok((req.run)()) },
        );

        let ok = run_input_on_main(|| Ok("clicked".to_string())).await;
        assert_eq!(ok, Ok("clicked".to_string()));

        let err = run_input_on_main(|| Err("enigo failed".to_string())).await;
        assert_eq!(err, Err("enigo failed".to_string()));
    }
}
