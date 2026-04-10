//! Generic graceful-shutdown facility for the core process.
//!
//! Provides a shutdown signal that listens for SIGINT (Ctrl-C) **and** SIGTERM
//! (on Unix), then runs registered cleanup hooks before the process exits.
//! Domain-specific cleanup (autocomplete, voice, etc.) registers itself here
//! so `jsonrpc.rs` stays transport-only.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use once_cell::sync::Lazy;

/// A boxed async cleanup function.
type ShutdownHook = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Global registry of shutdown hooks.
static HOOKS: Lazy<Mutex<Vec<ShutdownHook>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Register a cleanup function to run on graceful shutdown.
///
/// Hooks execute sequentially in registration order.
pub fn register<F, Fut>(hook: F)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let boxed: ShutdownHook = Box::new(move || Box::pin(hook()));
    HOOKS.lock().expect("shutdown hooks poisoned").push(boxed);
}

/// Run all registered hooks (called once during shutdown).
async fn run_hooks() {
    let hooks: Vec<ShutdownHook> = {
        let mut guard = HOOKS.lock().expect("shutdown hooks poisoned");
        std::mem::take(&mut *guard)
    };
    for hook in &hooks {
        hook().await;
    }
}

/// Returns a future that resolves when the process receives a termination
/// signal (SIGINT on all platforms, plus SIGTERM on Unix), then runs all
/// registered shutdown hooks.
pub async fn signal() {
    wait_for_signal().await;
    log::info!("[core] shutdown signal received, cleaning up background services");
    run_hooks().await;
    log::info!("[core] all shutdown hooks completed");
}

/// Wait for either SIGINT or SIGTERM (Unix) / just SIGINT (non-Unix).
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("[core] received SIGINT");
            }
            _ = sigterm.recv() => {
                log::info!("[core] received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("[core] received SIGINT");
    }
}
