//! Runtime safety utilities.
//!
//! This module provides a collection of safe execution wrappers designed to
//! prevent common asynchronous pitfalls in Rust, such as attempting to start
//! multiple Tokio runtimes on the same thread or causing deadlocks when
//! bridgeing between synchronous and asynchronous code.

use std::future::Future;
use tokio::runtime::{Handle, Runtime};

/// Safely executes an asynchronous future, automatically detecting the current context.
///
/// If an existing Tokio runtime handle is available in the current thread, the future
/// is spawned onto it, and the result is awaited via a synchronous channel.
/// If no runtime is active, a new single-threaded runtime is created to execute the future.
///
/// This is essential for bridge functions that may be called from both sync and async contexts.
///
/// # Errors
/// Returns an error string if a new runtime cannot be created or if result communication fails.
pub fn safe_async_execute<F, R>(future: F) -> Result<R, String>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    // Try to use existing runtime first
    if let Ok(handle) = Handle::try_current() {
        // We're in an async context, spawn on the current runtime
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        handle.spawn(async move {
            let result = future.await;
            let _ = tx.send(result);
        });

        rx.recv()
            .map_err(|e| format!("Failed to receive async result: {}", e))
    } else {
        // No runtime available, create a new one for this specific task
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create runtime: {}", e))?
            .block_on(async { Ok(future.await) })
    }
}

/// Safely executes an asynchronous future on a dedicated background thread.
///
/// This is particularly useful for avoiding deadlocks when executing async logic
/// from within contexts that block the current thread, such as certain V8 isolate
/// operations or synchronous bridge calls.
///
/// # Errors
/// Returns an error string if a runtime cannot be initialized on the new thread
/// or if the operation exceeds the 60-second timeout.
pub fn safe_async_execute_threaded<F, R>(future: F) -> Result<R, String>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    std::thread::spawn(move || {
        // Detect if a runtime was somehow inherited or initialized on this new thread
        if let Ok(handle) = Handle::try_current() {
            let result = handle.block_on(future);
            let _ = tx.send(Ok(result));
            return;
        }

        // Create a dedicated single-threaded runtime for this background operation
        let runtime_result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create runtime: {}", e))
            .map(|rt| rt.block_on(future));

        let result = match runtime_result {
            Ok(value) => Ok(value),
            Err(e) => Err(format!("Runtime creation failed: {}", e)),
        };

        let _ = tx.send(result);
    });

    rx.recv_timeout(std::time::Duration::from_secs(60))
        .map_err(|e| format!("Threaded async execution timed out: {}", e))?
}

/// Checks if the current thread is already executing within an active Tokio runtime.
pub fn is_in_runtime() -> bool {
    Handle::try_current().is_ok()
}

/// Attempts to create a new single-threaded Tokio runtime.
///
/// # Errors
/// Returns an error if the current thread already has an active runtime,
/// as Tokio does not support nested runtimes on the same thread.
pub fn create_runtime() -> Result<Runtime, String> {
    if is_in_runtime() {
        return Err("Cannot create runtime: already in async context".to_string());
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to create runtime: {}", e))
}

/// Executes a potentially blocking synchronous operation within an asynchronous context.
///
/// This offloads the operation to a thread pool managed by Tokio (`spawn_blocking`),
/// preventing it from stalling the async executor.
pub async fn safe_blocking<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("Blocking operation failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_async_execute_no_runtime() {
        let result = safe_async_execute(async { "test".to_string() });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_safe_async_execute_with_runtime() {
        let result = safe_async_execute(async { "test".to_string() });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test");
    }

    #[test]
    fn test_is_in_runtime() {
        // Outside async context
        assert!(!is_in_runtime());
    }

    #[tokio::test]
    async fn test_is_in_runtime_async() {
        // Inside async context
        assert!(is_in_runtime());
    }
}
