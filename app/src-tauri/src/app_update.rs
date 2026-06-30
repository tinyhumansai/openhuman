//! Bounded-retry policy for the Tauri auto-updater download.
//!
//! The updater fetches a large (~100MB) bundle from the GitHub release CDN.
//! A single transient mid-stream HTTP failure (`reqwest` "error decoding
//! response body", connection reset, incomplete body) otherwise aborts the
//! entire update with no retry — the root cause of Sentry TAURI-RUST-4JR
//! (2.8k events / 68 users): users could not update and re-triggered the
//! failing download repeatedly.
//!
//! We retry only on the transient network class (`Error::Reqwest`). Integrity
//! and config errors (`Minisign`, `SignatureUtf8`, `Io`, `TargetNotFound`, …)
//! are NOT retried — re-downloading cannot fix a bad signature or a missing
//! target, and looping on a verification failure could mask tampering.
//!
//! The download call sites live in `lib.rs` (`download_app_update` /
//! `apply_app_update`); they wrap the updater call in a loop driven by
//! [`classify`]. The decision logic is factored out here so it can be unit
//! tested without constructing a real `tauri_plugin_updater::Update` (which
//! needs a live endpoint) or a `reqwest::Error` (no public constructor).

use std::time::Duration;

/// Total download attempts before giving up (1 initial + 2 retries).
pub const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;

/// Outcome of evaluating a failed download attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Transient failure with attempts left — sleep [`backoff_for`] and retry.
    Retry,
    /// Fatal (non-transient) failure, or retry budget exhausted — surface the error.
    GiveUp,
}

/// Decide whether a failed attempt should be retried.
///
/// * `attempt` — 1-based index of the attempt that just failed.
/// * `max` — total attempt budget (see [`MAX_DOWNLOAD_ATTEMPTS`]).
/// * `transient` — whether the error is a retryable network error
///   (computed at the call site via [`is_transient_updater_err`]).
///
/// Pure function: no I/O, no error construction — fully unit-testable.
pub fn classify(attempt: u32, max: u32, transient: bool) -> RetryDecision {
    if transient && attempt < max {
        RetryDecision::Retry
    } else {
        RetryDecision::GiveUp
    }
}

/// Backoff before the next attempt: linear 2s · `attempt` (2s, 4s, …).
///
/// Kept short because the foreground (`apply_app_update`) path holds the core
/// shut down while it waits, and the user is actively waiting on the update.
pub fn backoff_for(attempt: u32) -> Duration {
    Duration::from_secs(2 * attempt as u64)
}

/// Whether an updater error is a transient network failure worth retrying.
///
/// Only [`tauri_plugin_updater::Error::Reqwest`] qualifies — that variant wraps
/// the `reqwest` transport/decode layer where "error decoding response body"
/// originates. Signature (`Minisign`/`SignatureUtf8`), filesystem (`Io`),
/// extract, and config (`TargetNotFound`/`ReleaseNotFound`) errors are
/// deliberately excluded.
pub fn is_transient_updater_err(err: &tauri_plugin_updater::Error) -> bool {
    matches!(err, tauri_plugin_updater::Error::Reqwest(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_with_attempts_left_retries() {
        assert_eq!(
            classify(1, MAX_DOWNLOAD_ATTEMPTS, true),
            RetryDecision::Retry
        );
        assert_eq!(
            classify(2, MAX_DOWNLOAD_ATTEMPTS, true),
            RetryDecision::Retry
        );
    }

    #[test]
    fn transient_on_last_attempt_gives_up() {
        assert_eq!(
            classify(MAX_DOWNLOAD_ATTEMPTS, MAX_DOWNLOAD_ATTEMPTS, true),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn transient_past_budget_gives_up() {
        assert_eq!(
            classify(MAX_DOWNLOAD_ATTEMPTS + 1, MAX_DOWNLOAD_ATTEMPTS, true),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn non_transient_never_retries() {
        // Fatal on the very first attempt, regardless of remaining budget.
        assert_eq!(
            classify(1, MAX_DOWNLOAD_ATTEMPTS, false),
            RetryDecision::GiveUp
        );
        assert_eq!(
            classify(2, MAX_DOWNLOAD_ATTEMPTS, false),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn single_attempt_budget_never_retries() {
        // max == 1 means no retries even for a transient error.
        assert_eq!(classify(1, 1, true), RetryDecision::GiveUp);
    }

    #[test]
    fn backoff_is_monotonic_and_bounded() {
        let b1 = backoff_for(1);
        let b2 = backoff_for(2);
        assert!(b2 > b1, "backoff must grow with attempt");
        assert_eq!(b1, Duration::from_secs(2));
        // Worst-case wait across the full budget stays small (2s + 4s = 6s).
        let total: Duration = (1..MAX_DOWNLOAD_ATTEMPTS).map(backoff_for).sum();
        assert!(
            total <= Duration::from_secs(10),
            "total backoff stays bounded"
        );
    }
}
