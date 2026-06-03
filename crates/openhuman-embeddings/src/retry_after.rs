//! Retry-After header parsing and 429/503 backoff logic for embedding clients.
//!
//! The HTTP `Retry-After` header may arrive as either:
//! - A non-negative integer: delta-seconds from now (e.g. `Retry-After: 30`)
//! - An HTTP-date string: absolute point in time (e.g. `Retry-After: Wed, 21 Oct 2015 07:28:00 GMT`)
//!
//! This module prefers the delta-seconds form and falls back to exponential
//! backoff when the header is absent or unparseable. See RFC 9110 §10.2.4.

/// Maximum number of 429/503 retries before giving up.
///
/// Three retries means the client makes up to four total attempts: the
/// original request plus three retries. This caps the per-call delay at
/// roughly `BASE_BACKOFF_MS * 2^2 = 4 s` in the no-`Retry-After` path (or
/// the server-directed duration in the header path).
pub const MAX_429_RETRIES: u32 = 3;

/// Base exponential-backoff delay in milliseconds when `Retry-After` is absent.
///
/// Sequence (per attempt): 1 s, 2 s, 4 s — capped at 30 s.
pub const BASE_BACKOFF_MS: u64 = 1_000;

/// Maximum backoff delay in milliseconds regardless of `Retry-After` or
/// computed exponent. Prevents a misbehaving server from parking the caller
/// for an unreasonably long time.
pub const MAX_BACKOFF_MS: u64 = 30_000;

/// Parse the `Retry-After` header value into a delay in milliseconds.
///
/// Accepts the delta-seconds form only (e.g. `"30"`, `"0"`). HTTP-date form
/// is not parsed — fall back to exponential backoff when the value is not a
/// non-negative integer.
///
/// Returns `None` when the header is absent, empty, or not a valid
/// non-negative integer.
pub fn parse_retry_after_ms(header_value: Option<&str>) -> Option<u64> {
    let s = header_value?.trim();
    // Only accept the delta-seconds form: a non-negative integer.
    let secs: u64 = s.parse().ok()?;
    Some(secs.saturating_mul(1_000).min(MAX_BACKOFF_MS))
}

/// Compute the delay for attempt `n` (0-indexed) with optional `Retry-After`
/// override. Uses exponential backoff as fallback:
/// `BASE_BACKOFF_MS * 2^n`, capped at `MAX_BACKOFF_MS`.
pub fn backoff_ms_for_attempt(attempt: u32, retry_after_header: Option<&str>) -> u64 {
    if let Some(ms) = parse_retry_after_ms(retry_after_header) {
        return ms;
    }
    BASE_BACKOFF_MS
        .saturating_mul(2u64.saturating_pow(attempt))
        .min(MAX_BACKOFF_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_retry_after_ms ────────────────────────────────────

    #[test]
    fn parses_integer_seconds() {
        assert_eq!(parse_retry_after_ms(Some("30")), Some(30_000));
    }

    #[test]
    fn parses_zero() {
        assert_eq!(parse_retry_after_ms(Some("0")), Some(0));
    }

    #[test]
    fn parses_whitespace_padded() {
        assert_eq!(parse_retry_after_ms(Some("  5  ")), Some(5_000));
    }

    #[test]
    fn caps_at_max_backoff() {
        // 9999 s * 1000 ms/s > MAX_BACKOFF_MS
        assert_eq!(parse_retry_after_ms(Some("9999")), Some(MAX_BACKOFF_MS));
    }

    #[test]
    fn returns_none_for_http_date() {
        // HTTP-date form — not parsed; fall through to exponential backoff.
        assert_eq!(
            parse_retry_after_ms(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
    }

    #[test]
    fn returns_none_for_none_input() {
        assert_eq!(parse_retry_after_ms(None), None);
    }

    #[test]
    fn returns_none_for_empty_string() {
        assert_eq!(parse_retry_after_ms(Some("")), None);
    }

    #[test]
    fn returns_none_for_negative() {
        // Negative integers are not valid delta-seconds per RFC 9110.
        assert_eq!(parse_retry_after_ms(Some("-1")), None);
    }

    // ── backoff_ms_for_attempt ─────────────────────────────────

    #[test]
    fn uses_retry_after_when_present() {
        assert_eq!(backoff_ms_for_attempt(0, Some("5")), 5_000);
        assert_eq!(backoff_ms_for_attempt(2, Some("5")), 5_000);
    }

    #[test]
    fn falls_back_to_exponential_when_header_absent() {
        assert_eq!(backoff_ms_for_attempt(0, None), BASE_BACKOFF_MS);
        assert_eq!(backoff_ms_for_attempt(1, None), BASE_BACKOFF_MS * 2);
        assert_eq!(backoff_ms_for_attempt(2, None), BASE_BACKOFF_MS * 4);
    }

    #[test]
    fn falls_back_to_exponential_when_header_unparseable() {
        assert_eq!(
            backoff_ms_for_attempt(0, Some("not-a-number")),
            BASE_BACKOFF_MS
        );
    }

    #[test]
    fn exponential_caps_at_max() {
        // 2^10 = 1024 — well past the cap
        assert_eq!(backoff_ms_for_attempt(10, None), MAX_BACKOFF_MS);
    }
}
