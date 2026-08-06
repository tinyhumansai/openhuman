//! Retry-with-backoff helpers and transient-filesystem-error classification.
//!
//! Primarily for Windows, where mandatory file locking makes
//! `ERROR_SHARING_VIOLATION` / `ERROR_ACCESS_DENIED` routine on a tree that
//! another process (or a stale handle) still holds.

/// Helper to retry a filesystem operation with exponential backoff.
///
/// Particularly useful on Windows where mandatory file locking often causes
/// transient `ERROR_SHARING_VIOLATION` (32) or `ERROR_ACCESS_DENIED` (5)
/// when multiple processes (or a stale handle) touch the same tree.
///
/// Sleep `base_ms * 2^i` between attempts. Logs at `warn!` on retry and
/// `info!` on success-after-retry.
///
/// **Note**: This is the synchronous version using `std::thread::sleep`.
/// Use `retry_with_backoff_async` in asynchronous contexts to avoid blocking
/// the executor.
pub fn retry_with_backoff<T, F>(
    op_name: &str,
    attempts: u32,
    base_ms: u64,
    mut f: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> anyhow::Result<T>,
{
    anyhow::ensure!(attempts > 0, "{} requires attempts > 0", op_name);

    let mut last_err: Option<anyhow::Error> = None;

    for i in 0..attempts {
        match f() {
            Ok(val) => {
                if i > 0 {
                    tracing::info!(op = op_name, retries = i, "[util] succeeded after retries");
                }
                return Ok(val);
            }
            Err(e) => {
                if !is_transient_fs_error(&e) {
                    return Err(e);
                }

                if i == attempts - 1 {
                    last_err = Some(e);
                    break;
                }

                let sleep_ms = base_ms.saturating_mul(2u64.saturating_pow(i)).min(30_000);
                tracing::warn!(
                    op = op_name,
                    attempt = i + 1,
                    max_attempts = attempts,
                    error = %e,
                    retry_in_ms = sleep_ms,
                    "[util] transient fs retry"
                );

                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
            }
        }
    }

    Err(last_err
        .expect("attempts > 0")
        .context(format!("{} failed after {} attempts", op_name, attempts)))
}

/// Asynchronous version of `retry_with_backoff` using `tokio::time::sleep`.
pub async fn retry_with_backoff_async<T, F, Fut>(
    op_name: &str,
    attempts: u32,
    base_ms: u64,
    mut f: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    anyhow::ensure!(attempts > 0, "{} requires attempts > 0", op_name);

    let mut last_err: Option<anyhow::Error> = None;

    for i in 0..attempts {
        match f().await {
            Ok(val) => {
                if i > 0 {
                    tracing::info!(op = op_name, retries = i, "[util] succeeded after retries");
                }
                return Ok(val);
            }
            Err(e) => {
                if !is_transient_fs_error(&e) {
                    return Err(e);
                }

                if i == attempts - 1 {
                    last_err = Some(e);
                    break;
                }

                let sleep_ms = base_ms.saturating_mul(2u64.saturating_pow(i)).min(30_000);
                tracing::warn!(
                    op = op_name,
                    attempt = i + 1,
                    max_attempts = attempts,
                    error = %e,
                    retry_in_ms = sleep_ms,
                    "[util] transient fs retry"
                );

                tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
            }
        }
    }

    Err(last_err
        .expect("attempts > 0")
        .context(format!("{} failed after {} attempts", op_name, attempts)))
}

/// Returns true if the error is a transient filesystem error that should be retried,
/// particularly on Windows where file locking is mandatory.
pub fn is_transient_fs_error(err: &anyhow::Error) -> bool {
    // In tests, allow a specific error message to be treated as transient
    // so we can verify the retry logic on all platforms.
    if cfg!(test) && err.to_string().contains("__TEST_TRANSIENT__") {
        return true;
    }

    let io_err = err.chain().find_map(|e| e.downcast_ref::<std::io::Error>());

    if let Some(io_err) = io_err {
        #[cfg(windows)]
        {
            if let Some(code) = io_err.raw_os_error() {
                // 5: ERROR_ACCESS_DENIED
                // 32: ERROR_SHARING_VIOLATION
                // 33: ERROR_LOCK_VIOLATION
                // 303: ERROR_DELETE_PENDING — the previous owner's
                //      `Drop::drop` issued `fs::remove_file` and Windows
                //      acknowledged it, but the file is still in the
                //      "delete pending" limbo because AV/indexer holds a
                //      handle. A retry-with-backoff resolves it as soon as
                //      the holder closes its handle. Sentry OPENHUMAN-TAURI-H8
                //      bails at `elapsed_ms ≈ 2` against
                //      `openhuman.team_get_usage` because this code was not
                //      previously classified as transient and `create_new`
                //      returned a `kind = Other` io::Error on the first try.
                // 665: ERROR_FILE_SYSTEM_LIMITATION — the NTFS filesystem
                //      is fragmented, the USN journal has overflowed, or a
                //      filter driver resource cap has been hit. Although
                //      not always transient (fragmentation is persistent),
                //      the USN journal and filter-driver cases can resolve
                //      after a delay, so exponential backoff is still
                //      better than an immediate bail + unthrottled outer
                //      retry. The observability module classifies persistent
                //      665 errors as `ExpectedErrorKind::WindowsFileSystemLimitation`
                //      to prevent Sentry flooding (TAURI-RUST-QT0).
                // 1224: ERROR_USER_MAPPED_FILE
                return code == 5
                    || code == 32
                    || code == 33
                    || code == 303
                    || code == 665
                    || code == 1224;
            }
        }
        #[cfg(not(windows))]
        {
            let _ = io_err;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_with_backoff_success_immediate() {
        let mut calls = 0;
        let result = retry_with_backoff("test", 3, 1, || {
            calls += 1;
            Ok::<_, anyhow::Error>(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls, 1);
    }

    #[test]
    fn test_retry_with_backoff_success_after_retries() {
        let mut calls = 0;
        let result = retry_with_backoff("test", 3, 1, || {
            calls += 1;
            if calls < 3 {
                anyhow::bail!("__TEST_TRANSIENT__ error {}", calls);
            }
            Ok(42)
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn test_retry_with_backoff_async_success_after_retries() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let result = retry_with_backoff_async("test_async", 3, 1, || async {
            let c = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if c < 3 {
                anyhow::bail!("__TEST_TRANSIENT__ error {}", c);
            }
            Ok(42)
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retry_with_backoff_failure_after_all_attempts() {
        let mut calls = 0;
        let result = retry_with_backoff("test", 3, 1, || {
            calls += 1;
            anyhow::bail!("__TEST_TRANSIENT__ error {}", calls);
            #[allow(unreachable_code)]
            Ok::<i32, anyhow::Error>(0)
        });
        let err = result.unwrap_err();
        assert!(err.to_string().contains("test failed after 3 attempts"));
        assert_eq!(calls, 3);
    }

    #[test]
    fn test_retry_with_backoff_bail_on_non_transient() {
        let mut calls = 0;
        let result = retry_with_backoff("test", 3, 1, || {
            calls += 1;
            anyhow::bail!("permanent error");
            #[allow(unreachable_code)]
            Ok::<i32, anyhow::Error>(0)
        });
        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "permanent error");
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_retry_with_backoff_async_bail_on_non_transient() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let result = retry_with_backoff_async("test_async_bail", 3, 1, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("permanent error");
            #[allow(unreachable_code)]
            Ok::<i32, anyhow::Error>(0)
        })
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "permanent error");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_retry_with_backoff_rejects_zero_attempts() {
        let mut calls = 0;
        let result = retry_with_backoff("zero_sync", 0, 1, || {
            calls += 1;
            Ok::<i32, anyhow::Error>(42)
        });
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("requires attempts > 0"),
            "unexpected error message: {}",
            err
        );
        assert_eq!(calls, 0, "closure must not run when attempts == 0");
    }

    #[tokio::test]
    async fn test_retry_with_backoff_async_rejects_zero_attempts() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let result = retry_with_backoff_async("zero_async", 0, 1, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, anyhow::Error>(42)
        })
        .await;
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("requires attempts > 0"),
            "unexpected error message: {}",
            err
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "closure must not run when attempts == 0"
        );
    }

    // ── is_transient_fs_error ──────────────────────────────────────

    /// The test-cfg backdoor: any error containing `__TEST_TRANSIENT__` is
    /// treated as transient so retry logic can be exercised on non-Windows
    /// CI runners without faking OS error codes.
    #[test]
    fn is_transient_fs_error_recognises_test_sentinel() {
        let err = anyhow::anyhow!("__TEST_TRANSIENT__ simulated lock violation");
        assert!(
            is_transient_fs_error(&err),
            "__TEST_TRANSIENT__ sentinel must be recognised as transient in test builds"
        );
    }

    /// A plain anyhow error (no io::Error chain) must not be treated as
    /// transient — the backoff must not swallow unknown failures.
    #[test]
    fn is_transient_fs_error_rejects_plain_anyhow_error() {
        let err = anyhow::anyhow!("some permanent application error");
        assert!(
            !is_transient_fs_error(&err),
            "plain anyhow error without IO chain must not be transient"
        );
    }

    #[cfg(windows)]
    #[test]
    fn is_transient_fs_error_classifies_windows_delete_pending() {
        let io_err = std::io::Error::from_raw_os_error(303);
        let err = anyhow::Error::new(io_err);
        assert!(
            is_transient_fs_error(&err),
            "ERROR_DELETE_PENDING (303) must be transient on Windows"
        );
    }

    /// A chained io::Error with `ErrorKind::NotFound` is not a transient
    /// locking error — we should not retry it.
    #[test]
    fn is_transient_fs_error_rejects_not_found_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = anyhow::Error::new(io_err);
        assert!(
            !is_transient_fs_error(&err),
            "NotFound IO error must not be transient"
        );
    }

    /// Verify that retry_with_backoff retries exactly when the test
    /// sentinel is present and bails immediately on a non-transient error.
    /// This exercises the `is_transient_fs_error` integration path.
    #[test]
    fn retry_with_backoff_respects_transient_classification() {
        let mut calls = 0usize;

        // Transient path: retries until success.
        let result = retry_with_backoff("transient_class", 3, 1, || {
            calls += 1;
            if calls < 2 {
                anyhow::bail!("__TEST_TRANSIENT__ lock error");
            }
            Ok(calls)
        });
        assert_eq!(result.unwrap(), 2, "should succeed on second attempt");
        assert_eq!(calls, 2, "must have retried once");

        // Non-transient path: bails after first attempt.
        let mut calls2 = 0usize;
        let result2 = retry_with_backoff("non_transient_class", 3, 1, || {
            calls2 += 1;
            anyhow::bail!("hard permanent error");
            #[allow(unreachable_code)]
            Ok::<_, anyhow::Error>(())
        });
        assert!(result2.is_err(), "non-transient must fail");
        assert_eq!(calls2, 1, "must NOT retry a non-transient error");
    }
}
