//! Runtime safety utilities.
//!
//! Provides safe ways to execute async code in different runtime contexts,
//! preventing the "Cannot start a runtime from within a runtime" panic.

use std::future::Future;
use tokio::runtime::{Handle, Runtime};

/// Safely execute async code, trying to use existing runtime first.
///
/// This function attempts to use the current async runtime handle if available,
/// otherwise creates a new single-threaded runtime. This prevents the common
/// panic "Cannot start a runtime from within a runtime".
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
        // No runtime available, create a new one
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create runtime: {}", e))?
            .block_on(async { Ok(future.await) })
    }
}

/// Safely execute async code on a separate thread to avoid deadlocks.
///
/// This is useful when calling async functions from within V8 isolates or
/// other contexts where blocking the current thread would cause deadlocks.
pub fn safe_async_execute_threaded<F, R>(future: F) -> Result<R, String>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    std::thread::spawn(move || {
        // Try using existing runtime first
        if let Ok(handle) = Handle::try_current() {
            let result = handle.block_on(future);
            let _ = tx.send(Ok(result));
            return;
        }

        // Create new single-threaded runtime if no runtime exists
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

/// Check if we're currently in an async runtime context.
pub fn is_in_runtime() -> bool {
    Handle::try_current().is_ok()
}

/// Create a new single-threaded runtime safely.
///
/// Returns an error if a runtime is already active in the current thread.
pub fn create_runtime() -> Result<Runtime, String> {
    if is_in_runtime() {
        return Err("Cannot create runtime: already in async context".to_string());
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to create runtime: {}", e))
}

/// Execute a blocking operation safely within an async context.
///
/// Uses spawn_blocking to avoid blocking the async executor.
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
