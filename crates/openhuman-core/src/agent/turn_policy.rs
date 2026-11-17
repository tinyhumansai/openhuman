//! Shared wall-clock policy for a user-facing agent turn.
//!
//! The idle guard catches turns that stop making progress; the independent
//! backstop lets a turn remain active while it is still producing progress.

use std::time::Duration;

/// Default silence window for a top-level user turn.
pub(crate) const DEFAULT_IDLE_SECS: u64 = 900;

/// Existing environment override retained for compatibility with deployments.
pub(crate) const IDLE_ENV: &str = "OPENHUMAN_WEB_TURN_IDLE_SECS";

const IDLE_DISABLED: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// Parse an idle window; zero disables the guard and invalid values use the
/// product default.
pub(crate) fn parse_idle_window(value: Option<&str>) -> Duration {
    let secs = value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_IDLE_SECS);
    if secs == 0 {
        IDLE_DISABLED
    } else {
        Duration::from_secs(secs)
    }
}

/// The process-wide idle window for top-level turns.
pub(crate) fn configured_idle_window() -> Duration {
    parse_idle_window(std::env::var(IDLE_ENV).ok().as_deref())
}
