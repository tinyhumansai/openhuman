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
