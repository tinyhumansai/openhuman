//! Configurable HTTP timeouts for the OpenAI-compatible inference provider.
//!
//! The request and connect timeouts used when talking to inference endpoints
//! were hardcoded (120s / 10s) in [`super::compatible_request`], which cut off
//! long reasoning/research turns at the two-minute mark (#3856). They are now
//! resolved from environment variables, with the previous values as defaults so
//! behaviour is unchanged unless an operator overrides them:
//!
//!   - `OPENHUMAN_INFERENCE_TIMEOUT_SECS`         — whole-request timeout (default 120)
//!   - `OPENHUMAN_INFERENCE_CONNECT_TIMEOUT_SECS` — connection-establishment timeout (default 10)
//!
//! A missing, non-numeric, or out-of-range value falls back to the default
//! (logged at debug level by [`resolve`]), so a typo can never disable the
//! timeout or wedge a turn indefinitely.

use std::sync::OnceLock;
use std::time::Duration;

/// Default whole-request timeout in seconds (preserves the prior hardcoded value).
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 120;
/// Default connection-establishment timeout in seconds (preserves the prior value).
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Smallest accepted timeout. `0` would disable the timeout entirely, so it is
/// rejected and falls back to the default.
const MIN_TIMEOUT_SECS: u64 = 1;
/// Largest accepted request timeout (1 hour) — guards against typos that would
/// let a hung request wedge a session indefinitely.
const MAX_REQUEST_TIMEOUT_SECS: u64 = 3600;
/// Largest accepted connect timeout (5 minutes) — establishing a connection
/// should never legitimately take longer.
const MAX_CONNECT_TIMEOUT_SECS: u64 = 300;

const REQUEST_ENV_VAR: &str = "OPENHUMAN_INFERENCE_TIMEOUT_SECS";
const CONNECT_ENV_VAR: &str = "OPENHUMAN_INFERENCE_CONNECT_TIMEOUT_SECS";

/// Parse a raw env-var value into a bounded timeout in seconds.
///
/// Pure (no env / global access) so unit tests can exercise every branch
/// without mutating the process environment or racing other tests. `None`,
/// non-numeric, or values outside `min..=max` return `default`.
fn parse_timeout_secs(raw: Option<&str>, default: u64, min: u64, max: u64) -> u64 {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| (min..=max).contains(n))
        .unwrap_or(default)
}

/// Resolve an env-configured timeout. When the var is set, the resolved value
/// is logged so an operator can tell whether an invalid override silently fell
/// back to the default.
fn resolve(env_var: &str, default: u64, max: u64) -> Duration {
    let raw = std::env::var(env_var).ok();
    let secs = parse_timeout_secs(raw.as_deref(), default, MIN_TIMEOUT_SECS, max);
    if let Some(value) = raw.as_deref() {
        tracing::debug!(
            "[inference] {env_var}={value:?} -> {secs}s (allowed {MIN_TIMEOUT_SECS}..={max}, default {default})"
        );
    }
    Duration::from_secs(secs)
}

/// Whole-request timeout for inference HTTP calls.
/// Override via `OPENHUMAN_INFERENCE_TIMEOUT_SECS` (default 120s, range 1..=3600).
///
/// `http_client()` is rebuilt on every inference request (80+ call sites), so
/// the value is resolved once per process and cached — env vars don't change at
/// runtime, and this keeps the hot path off `std::env::var` and avoids logging
/// the resolution on every request (mirrors `tool_timeout`'s cached value).
pub(super) fn request_timeout() -> Duration {
    static CACHED: OnceLock<Duration> = OnceLock::new();
    *CACHED.get_or_init(|| {
        resolve(
            REQUEST_ENV_VAR,
            DEFAULT_REQUEST_TIMEOUT_SECS,
            MAX_REQUEST_TIMEOUT_SECS,
        )
    })
}

/// Connection-establishment timeout for inference HTTP calls.
/// Override via `OPENHUMAN_INFERENCE_CONNECT_TIMEOUT_SECS` (default 10s, range 1..=300).
/// Resolved once per process and cached — see [`request_timeout`].
pub(super) fn connect_timeout() -> Duration {
    static CACHED: OnceLock<Duration> = OnceLock::new();
    *CACHED.get_or_init(|| {
        resolve(
            CONNECT_ENV_VAR,
            DEFAULT_CONNECT_TIMEOUT_SECS,
            MAX_CONNECT_TIMEOUT_SECS,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_default_when_absent_or_unparseable() {
        assert_eq!(parse_timeout_secs(None, 120, 1, 3600), 120);
        assert_eq!(parse_timeout_secs(Some(""), 120, 1, 3600), 120);
        assert_eq!(parse_timeout_secs(Some("   "), 120, 1, 3600), 120);
        assert_eq!(parse_timeout_secs(Some("abc"), 120, 1, 3600), 120);
        assert_eq!(parse_timeout_secs(Some("12.5"), 120, 1, 3600), 120);
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert_eq!(parse_timeout_secs(Some("0"), 120, 1, 3600), 120); // below min disables timeout
        assert_eq!(parse_timeout_secs(Some("99999"), 120, 1, 3600), 120); // above max
        assert_eq!(parse_timeout_secs(Some("301"), 10, 1, 300), 10); // connect ceiling
    }

    #[test]
    fn accepts_in_range_values_and_boundaries() {
        assert_eq!(parse_timeout_secs(Some("600"), 120, 1, 3600), 600);
        assert_eq!(parse_timeout_secs(Some(" 45 "), 120, 1, 3600), 45); // surrounding whitespace
        assert_eq!(parse_timeout_secs(Some("1"), 120, 1, 3600), 1); // min boundary
        assert_eq!(parse_timeout_secs(Some("3600"), 120, 1, 3600), 3600); // max boundary
    }

    #[test]
    fn default_constants_match_the_prior_hardcoded_values() {
        // The getters must return the exact previous behaviour when nothing is
        // overridden, so an unconfigured install is byte-for-byte unchanged.
        assert_eq!(DEFAULT_REQUEST_TIMEOUT_SECS, 120);
        assert_eq!(DEFAULT_CONNECT_TIMEOUT_SECS, 10);
    }
}
