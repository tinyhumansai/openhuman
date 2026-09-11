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
pub type ShutdownHook = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Global registry of shutdown hooks.
static HOOKS: Lazy<Mutex<Vec<ShutdownHook>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Register a cleanup function to run on graceful shutdown.
///
/// Use this to perform necessary cleanup tasks such as stopping background
/// services, flushing caches, or closing database connections when the
/// application is shutting down.
///
/// Hooks execute sequentially in the order they were registered.
///
/// # Arguments
///
/// * `hook` - A function that returns a future. The future will be awaited
///   during the shutdown process.
pub fn register<F, Fut>(hook: F)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    HOOKS
        .lock()
        .expect("shutdown hooks poisoned")
        .push(boxed_hook(hook));
}

/// Box a cleanup function without registering it.
///
/// For a caller that wants to run a hook through [`run_hook_list`] itself —
/// a test above all, which must not reach into the process-wide registry: in
/// a test binary that registry holds every other test's hooks, the shared
/// memory engine's shutdown among them.
pub fn boxed_hook<F, Fut>(hook: F) -> ShutdownHook
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Box::new(move || Box::pin(hook()))
}

/// Take every registered hook, leaving the registry empty.
///
/// Draining is what makes the exit idempotent: a later take, or a signal
/// landing mid-teardown, finds nothing to run twice.
pub fn take_hooks() -> Vec<ShutdownHook> {
    let mut guard = HOOKS.lock().expect("shutdown hooks poisoned");
    std::mem::take(&mut *guard)
}

/// Run all registered hooks (called once during shutdown).
///
/// This function drains the global `HOOKS` list and awaits each hook in sequence.
async fn run_hooks() {
    for hook in &take_hooks() {
        hook().await;
    }
}

/// Run `hooks` side by side.
///
/// For the embedded server's exit, whose graceful path is a cancellation token
/// rather than SIGTERM: [`signal`] never resolves there, so the hooks it would
/// have run — the memory engine releasing its queue leases, above all — never
/// ran on a normal quit. Concurrent, unlike the signal path, because the caller
/// runs this under a deadline, and in sequence one hook that never answers
/// would keep every hook after it — the lease release among them — from so
/// much as starting before the deadline dropped the lot. Run together, a
/// hanging hook costs only itself.
pub async fn run_hook_list(hooks: Vec<ShutdownHook>) {
    futures::future::join_all(hooks.iter().map(|hook| hook())).await;
}

/// Returns a future that resolves when the process receives a termination
/// signal (SIGINT on all platforms, plus SIGTERM on Unix), then runs all
/// registered shutdown hooks.
///
/// This is intended to be used with `axum::serve`'s `with_graceful_shutdown`
/// method or in the main loop to handle clean exits. (Plain code span, not an
/// intra-doc link: the direct `axum` dependency/API surface is unavailable in
/// slim builds — the `http-server` feature (#5048) gates it, and it remains
/// only transitively via `tinychannels` — where an intra-doc link to it would
/// fail rustdoc.)
pub async fn signal() {
    // Wait for the OS to send a termination signal.
    wait_for_signal().await;
    log::info!("[core] shutdown signal received, cleaning up background services");
    // Once received, run all registered cleanup tasks.
    run_hooks().await;
    log::info!("[core] all shutdown hooks completed");
}

/// Wait for either SIGINT (Ctrl-C) or SIGTERM (Unix termination signal).
///
/// This uses `tokio::signal` to asynchronously wait for these events.
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("[core] received SIGINT (Ctrl-C)");
            }
            _ = sigterm.recv() => {
                log::info!("[core] received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        // On non-Unix platforms (like Windows), we only listen for Ctrl-C.
        let _ = tokio::signal::ctrl_c().await;
        log::info!("[core] received SIGINT (Ctrl-C)");
    }
}
