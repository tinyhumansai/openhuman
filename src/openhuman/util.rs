//! Utility functions for `OpenHuman`.
//!
//! This module contains reusable helper functions used across the codebase.

/// Truncate a string to at most `max_chars` characters, appending "..." if truncated.
///
/// This function safely handles multi-byte UTF-8 characters (emoji, CJK, accented characters)
/// by using character boundaries instead of byte indices.
///
/// # Arguments
/// * `s` - The string to truncate
/// * `max_chars` - Maximum number of characters to keep (excluding "...")
///
/// # Returns
/// * Original string if length <= `max_chars`
/// * Truncated string with "..." appended if length > `max_chars`
///
/// # Examples
/// ```
/// use openhuman_core::openhuman::util::truncate_with_ellipsis;
///
/// // ASCII string - no truncation needed
/// assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
///
/// // ASCII string - truncation needed
/// assert_eq!(truncate_with_ellipsis("hello world", 5), "hello...");
///
/// // Multi-byte UTF-8 (emoji) - safe truncation
/// assert_eq!(truncate_with_ellipsis("Hello 🦀 World", 8), "Hello 🦀...");
/// assert_eq!(truncate_with_ellipsis("😀😀😀😀", 2), "😀😀...");
///
/// // Empty string
/// assert_eq!(truncate_with_ellipsis("", 10), "");
/// ```
pub fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    truncate_with_suffix(s, max_chars, "...")
}

/// Truncate a string to at most `max_chars` characters, appending `suffix` if truncated.
pub fn truncate_with_suffix(s: &str, max_chars: usize, suffix: &str) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => {
            let truncated = &s[..idx];
            // Trim trailing whitespace for cleaner output
            format!("{}{}", truncated.trim_end(), suffix)
        }
        None => s.to_string(),
    }
}

/// Truncate a string to at most `max_bytes` bytes, appending a single-character
/// ellipsis `…` (3 bytes) if truncated. The returned string's total byte
/// length will never exceed `max_bytes`.
pub fn truncate_at_byte_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let ellipsis = "…";
    let ellipsis_len = ellipsis.len();
    if max_bytes < ellipsis_len {
        return String::new();
    }
    let mut end = max_bytes - ellipsis_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &s[..end], ellipsis)
}

/// Round a byte index DOWN to the nearest UTF-8 character boundary.
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut end = index;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Return a prefix of `s` whose byte length is at most `max_bytes`, backing up
/// to the nearest UTF-8 character boundary when `max_bytes` falls in the middle
/// of a multi-byte character.
pub fn utf8_safe_prefix_at_byte_boundary(s: &str, max_bytes: usize) -> &str {
    &s[..floor_char_boundary(s, max_bytes)]
}

/// Round a byte index UP to the nearest UTF-8 character boundary.
pub fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut start = index;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    start
}

/// Utility enum for handling optional values.
pub enum MaybeSet<T> {
    Set(T),
    Unset,
    Null,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ascii_no_truncation() {
        // ASCII string shorter than limit - no change
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_with_ellipsis("hello world", 50), "hello world");
    }

    #[test]
    fn test_truncate_ascii_with_truncation() {
        // ASCII string longer than limit - truncates
        assert_eq!(truncate_with_ellipsis("hello world", 5), "hello...");
        assert_eq!(
            truncate_with_ellipsis("This is a long message", 10),
            "This is a..."
        );
    }

    #[test]
    fn test_truncate_empty_string() {
        assert_eq!(truncate_with_ellipsis("", 10), "");
    }

    #[test]
    fn test_truncate_at_exact_boundary() {
        // String exactly at boundary - no truncation
        assert_eq!(truncate_with_ellipsis("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_emoji_single() {
        // Single emoji (4 bytes) - should not panic
        let s = "🦀";
        assert_eq!(truncate_with_ellipsis(s, 10), s);
        assert_eq!(truncate_with_ellipsis(s, 1), s);
    }

    #[test]
    fn test_truncate_emoji_multiple() {
        // Multiple emoji - safe truncation at character boundary
        let s = "😀😀😀😀"; // 4 emoji, each 4 bytes = 16 bytes total
        assert_eq!(truncate_with_ellipsis(s, 2), "😀😀...");
        assert_eq!(truncate_with_ellipsis(s, 3), "😀😀😀...");
    }

    #[test]
    fn test_truncate_mixed_ascii_emoji() {
        // Mixed ASCII and emoji
        assert_eq!(truncate_with_ellipsis("Hello 🦀 World", 8), "Hello 🦀...");
        assert_eq!(truncate_with_ellipsis("Hi 😊", 10), "Hi 😊");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        // CJK characters (Chinese - each is 3 bytes)
        let s = "这是一个测试消息用来触发崩溃 of the 中文"; // 21 characters
        let result = truncate_with_ellipsis(s, 16);
        assert!(result.ends_with("..."));
        assert!(result.is_char_boundary(result.len() - 1));
    }

    #[test]
    fn test_truncate_accented_characters() {
        // Accented characters (2 bytes each in UTF-8)
        let s = "café résumé naïve";
        assert_eq!(truncate_with_ellipsis(s, 10), "café résum...");
    }

    #[test]
    fn test_truncate_unicode_edge_case() {
        // Mix of 1-byte, 2-byte, 3-byte, and 4-byte characters
        let s = "aé你好🦀"; // 1 + 1 + 2 + 2 + 4 bytes = 10 bytes, 5 chars
        assert_eq!(truncate_with_ellipsis(s, 3), "aé你...");
    }

    #[test]
    fn test_truncate_long_string() {
        // Long ASCII string
        let s = "a".repeat(200);
        let result = truncate_with_ellipsis(&s, 50);
        assert_eq!(result.len(), 53); // 50 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_zero_max_chars() {
        // Edge case: max_chars = 0
        assert_eq!(truncate_with_ellipsis("hello", 0), "...");
    }

    #[test]
    fn test_truncate_at_byte_boundary() {
        let s = "Hello 🦀 World"; // 16 bytes total. "🦀" is 4 bytes at index 6-9.
                                  // No truncation
        assert_eq!(truncate_at_byte_boundary(s, 16), s);
        assert_eq!(truncate_at_byte_boundary(s, 20), s);

        // Truncate at index 11 (the space after 🦀)
        // max_bytes = 14, ellipsis = 3 bytes, target end = 11.
        assert_eq!(truncate_at_byte_boundary(s, 14), "Hello 🦀 …");

        // Truncate mid-emoji (byte 8 is mid-🦀)
        // max_bytes = 9, ellipsis = 3 bytes, target end = 6.
        // should back up to index 6, add "…" (3 bytes) -> 9 bytes total
        let truncated = truncate_at_byte_boundary(s, 9);
        assert_eq!(truncated, "Hello …");
        assert!(truncated.len() <= 9);

        // Very small budget
        assert_eq!(truncate_at_byte_boundary("abc", 2), "");
        assert_eq!(truncate_at_byte_boundary("abc", 3), "abc");
    }

    #[test]
    fn test_floor_char_boundary() {
        let s = "A🦀C";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 1); // After 'A'
        assert_eq!(floor_char_boundary(s, 2), 1); // Mid-🦀
        assert_eq!(floor_char_boundary(s, 3), 1); // Mid-🦀
        assert_eq!(floor_char_boundary(s, 4), 1); // Mid-🦀
        assert_eq!(floor_char_boundary(s, 5), 5); // After '🦀'
        assert_eq!(floor_char_boundary(s, 6), 6); // After 'C'
        assert_eq!(floor_char_boundary(s, 100), 6);
    }

    #[test]
    fn test_utf8_safe_prefix_at_byte_boundary() {
        let s = format!("{}{}tail", "a".repeat(79), "魔");
        assert_eq!(utf8_safe_prefix_at_byte_boundary(&s, 80), "a".repeat(79));
        assert_eq!(utf8_safe_prefix_at_byte_boundary(&s, s.len()), s);
        assert_eq!(
            utf8_safe_prefix_at_byte_boundary("ascii preview", 5),
            "ascii"
        );
        assert_eq!(utf8_safe_prefix_at_byte_boundary("short", 80), "short");

        for cap in [30, 40, 80, 200, 500] {
            let preview = format!("{}{}tail", "a".repeat(cap - 1), "界");
            let truncated = utf8_safe_prefix_at_byte_boundary(&preview, cap);
            assert_eq!(truncated, "a".repeat(cap - 1));
            assert!(preview.is_char_boundary(truncated.len()));
        }
    }

    #[test]
    fn test_ceil_char_boundary() {
        let s = "A🦀C";
        assert_eq!(ceil_char_boundary(s, 0), 0);
        assert_eq!(ceil_char_boundary(s, 1), 1); // After 'A'
        assert_eq!(ceil_char_boundary(s, 2), 5); // Mid-🦀
        assert_eq!(ceil_char_boundary(s, 3), 5); // Mid-🦀
        assert_eq!(ceil_char_boundary(s, 4), 5); // Mid-🦀
        assert_eq!(ceil_char_boundary(s, 5), 5); // After '🦀'
        assert_eq!(ceil_char_boundary(s, 6), 6); // After 'C'
        assert_eq!(ceil_char_boundary(s, 100), 6);
    }

    #[test]
    fn test_truncate_with_suffix() {
        let s = "Hello World";
        assert_eq!(truncate_with_suffix(s, 5, "!!!"), "Hello!!!");
        assert_eq!(truncate_with_suffix(s, 20, "!!!"), "Hello World");
    }

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
                // 1224: ERROR_USER_MAPPED_FILE
                return code == 5 || code == 32 || code == 33 || code == 1224;
            }
        }
        #[cfg(not(windows))]
        {
            let _ = io_err;
        }
    }
    false
}
