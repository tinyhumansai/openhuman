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
#[path = "retry_tests.rs"]
mod tests;
