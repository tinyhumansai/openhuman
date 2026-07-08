//! Configurable HTTP timeouts for the OpenAI-compatible inference provider.
//!
//! The request and connect timeouts used when talking to inference endpoints
//! were hardcoded (120s / 10s) in [`super::compatible_request`], which cut off
//! long reasoning/research turns at the two-minute mark (#3856). They are now
//! resolved from environment variables, with the previous values as defaults so
//! behaviour is unchanged unless an operator overrides them:
//!
//!   - `OPENHUMAN_INFERENCE_TIMEOUT_SECS`            — whole-request timeout (default 120)
//!   - `OPENHUMAN_INFERENCE_CONNECT_TIMEOUT_SECS`    — connection-establishment timeout (default 10)
//!   - `OPENHUMAN_INFERENCE_STREAM_IDLE_TIMEOUT_SECS` — per-chunk stream inactivity timeout (default 90, #4269)
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
/// Default per-chunk stream inactivity timeout in seconds (#4269). Sits
/// comfortably above normal inter-token gaps — including reasoning-model
/// thinking pauses, which still stream as `reasoning_content` deltas and so
/// reset the window — yet below the 120s whole-request default, so a stalled
/// RESPONSE phase is caught and retried rather than held to the request ceiling
/// (up to 1 hour). The window RESETS on every received chunk, so a legitimately
/// long answer that keeps emitting tokens is never cut.
const DEFAULT_STREAM_IDLE_TIMEOUT_SECS: u64 = 90;
/// Largest accepted stream-idle timeout (1 hour) — matches the request ceiling.
const MAX_STREAM_IDLE_TIMEOUT_SECS: u64 = 3600;

const REQUEST_ENV_VAR: &str = "OPENHUMAN_INFERENCE_TIMEOUT_SECS";
const CONNECT_ENV_VAR: &str = "OPENHUMAN_INFERENCE_CONNECT_TIMEOUT_SECS";
const STREAM_IDLE_ENV_VAR: &str = "OPENHUMAN_INFERENCE_STREAM_IDLE_TIMEOUT_SECS";

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

/// Per-chunk inactivity timeout for streaming inference responses (#4269).
/// Override via `OPENHUMAN_INFERENCE_STREAM_IDLE_TIMEOUT_SECS` (default 90s,
/// range 1..=3600). The streaming read loop and each downstream delta send are
/// guarded by this window; it RESETS on every received chunk, so a legitimately
/// long response that keeps emitting tokens is never cut — only a genuine stall
/// (no bytes from upstream, or a wedged consumer) for the whole window trips it.
/// Resolved once per process and cached — see [`request_timeout`].
pub(super) fn stream_idle_timeout() -> Duration {
    static CACHED: OnceLock<Duration> = OnceLock::new();
    *CACHED.get_or_init(|| {
        resolve(
            STREAM_IDLE_ENV_VAR,
            DEFAULT_STREAM_IDLE_TIMEOUT_SECS,
            MAX_STREAM_IDLE_TIMEOUT_SECS,
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

    #[test]
    fn stream_idle_default_fires_before_the_whole_request_timeout() {
        // #4269: the watchdog must trip before the whole-request deadline so a
        // stalled RESPONSE phase is retried, not held to the request ceiling.
        assert_eq!(DEFAULT_STREAM_IDLE_TIMEOUT_SECS, 90);
        assert!(DEFAULT_STREAM_IDLE_TIMEOUT_SECS < DEFAULT_REQUEST_TIMEOUT_SECS);
    }

    #[test]
    fn stream_idle_parse_respects_bounds() {
        let (def, min, max) = (DEFAULT_STREAM_IDLE_TIMEOUT_SECS, MIN_TIMEOUT_SECS, 3600);
        assert_eq!(parse_timeout_secs(None, def, min, max), def);
        assert_eq!(parse_timeout_secs(Some("0"), def, min, max), def); // 0 would disable
        assert_eq!(parse_timeout_secs(Some("5"), def, min, max), 5);
        assert_eq!(parse_timeout_secs(Some("3600"), def, min, max), max); // ceiling
        assert_eq!(parse_timeout_secs(Some("3601"), def, min, max), def); // above ceiling
    }

    #[test]
    fn stream_idle_getter_returns_a_sane_duration() {
        // Unset (or set) in the env, the cached getter must resolve to a value
        // inside the documented bounds — never 0 (which would disable the guard).
        let d = stream_idle_timeout();
        assert!(d.as_secs() >= MIN_TIMEOUT_SECS && d.as_secs() <= MAX_STREAM_IDLE_TIMEOUT_SECS);
    }
}
