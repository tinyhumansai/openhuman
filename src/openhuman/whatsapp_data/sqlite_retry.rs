//! SQLite busy/locked detection and retry-with-backoff for WhatsApp data writes.
//!
//! Modelled on [`crate::openhuman::memory_queue::worker::is_sqlite_busy`] —
//! the configured `busy_timeout` absorbs short waits inside rusqlite; this layer
//! catches residual `SQLITE_BUSY` / `SQLITE_LOCKED` after that window.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

/// Per-connection busy handler window (issue #2077).
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Application-level retries after rusqlite's busy handler is exhausted.
const WRITE_RETRY_ATTEMPTS: u32 = 6;
const WRITE_RETRY_BASE_MS: u64 = 25;

/// Returns true when `err` is transient SQLite write-lock contention.
pub fn is_sqlite_busy(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        return matches!(
            sqlite_err.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        );
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database is locked") || msg.contains("database table is locked")
}

/// Run `f` up to [`WRITE_RETRY_ATTEMPTS`] times when SQLite reports busy/locked.
pub fn retry_on_sqlite_busy<T, F>(op_name: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..WRITE_RETRY_ATTEMPTS {
        match f() {
            Ok(val) => {
                if attempt > 0 {
                    log::debug!("[whatsapp_data] {op_name} succeeded after {attempt} busy retries");
                }
                return Ok(val);
            }
            Err(e) => {
                if !is_sqlite_busy(&e) {
                    return Err(e);
                }
                if attempt + 1 == WRITE_RETRY_ATTEMPTS {
                    last_err = Some(e);
                    break;
                }
                let sleep_ms = WRITE_RETRY_BASE_MS
                    .saturating_mul(2u64.saturating_pow(attempt))
                    .min(500);
                log::warn!(
                    "[whatsapp_data] {op_name} SQLite busy/locked \
                     (attempt {} of {WRITE_RETRY_ATTEMPTS}), retry in {sleep_ms}ms: {e:#}",
                    attempt + 1,
                );
                thread::sleep(Duration::from_millis(sleep_ms));
            }
        }
    }

    Err(last_err.expect("WRITE_RETRY_ATTEMPTS > 0").context(format!(
        "{op_name} failed after {WRITE_RETRY_ATTEMPTS} SQLite busy retries"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_sqlite_busy_matches_database_busy_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".into()),
        );
        let err = anyhow::Error::from(raw);
        assert!(is_sqlite_busy(&err));
    }

    #[test]
    fn is_sqlite_busy_does_not_match_constraint_violation() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 19,
            },
            Some("UNIQUE constraint failed".into()),
        );
        let err = anyhow::Error::from(raw);
        assert!(!is_sqlite_busy(&err));
    }

    #[test]
    fn retry_on_sqlite_busy_succeeds_after_transient_busy() {
        let mut calls = 0u32;
        let result = retry_on_sqlite_busy("test_op", || {
            calls += 1;
            if calls < 3 {
                Err(anyhow::Error::from(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error {
                        code: rusqlite::ErrorCode::DatabaseBusy,
                        extended_code: 5,
                    },
                    Some("database is locked".into()),
                )))
            } else {
                Ok(42usize)
            }
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_on_sqlite_busy_does_not_retry_non_busy_errors() {
        let mut calls = 0u32;
        let result: Result<()> = retry_on_sqlite_busy("test_op", || {
            calls += 1;
            anyhow::bail!("permanent failure");
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
