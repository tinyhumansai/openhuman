//! PII redaction helpers for log output.
//!
//! Per project rule (CLAUDE.md): "Never log secrets or full PII."
//! After the participant-bucketing change introduced in the MD-content PR,
//! source_ids and content_paths can embed full email addresses, so any log
//! line that prints them needs to redact.

use sha2::{Digest, Sha256};

/// Redact a string by hashing it to 8 hex chars. Stable across runs for the
/// same input — safe to grep for in logs when debugging with the raw value
/// available externally.
///
/// Use for source_ids, entity_ids, content_paths and similar PII-bearing
/// strings in log output.
pub fn redact(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let d = h.finalize();
    format!("{:08x}", u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
}

/// Redact a URL/endpoint by stripping path, query, fragment and credentials,
/// keeping only the host (and port if present).
///
/// Examples:
/// - `"http://localhost:11434/api/chat"` → `"localhost:11434"`
/// - `"https://user:pass@example.com/foo?q=1"` → `"example.com"`
/// - `"ollama://host:1234"` → `"host:1234"`
///
/// Does not pull in a URL-parsing crate; uses cheap string splitting which is
/// sufficient for the endpoint-config strings this codebase passes around.
pub fn redact_endpoint(url: &str) -> String {
    // Strip scheme (everything before "://").
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    // Strip credentials (everything before the last "@" before the first "/").
    let after_creds = after_scheme
        .split_once('@')
        .map(|(_, r)| r)
        .unwrap_or(after_scheme);
    // Take only the host:port part (up to the first '/', '?', or '#').
    let host_port = after_creds
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_creds);
    host_port.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── redact ───────────────────────────────────────────────────────────────

    #[test]
    fn redact_returns_eight_hex_chars() {
        let r = redact("alice@example.com");
        assert_eq!(r.len(), 8, "must be 8 hex chars; got {r:?}");
        assert!(r.chars().all(|c| c.is_ascii_hexdigit()), "must be hex");
    }

    #[test]
    fn redact_is_stable_across_calls() {
        assert_eq!(redact("alice@example.com"), redact("alice@example.com"));
    }

    #[test]
    fn redact_is_different_for_different_inputs() {
        assert_ne!(redact("alice@example.com"), redact("bob@example.com"));
    }

    #[test]
    fn redact_empty_string_does_not_panic() {
        let r = redact("");
        assert_eq!(r.len(), 8);
    }

    // ── redact_endpoint ─────────────────────────────────────────────────────

    #[test]
    fn redact_endpoint_strips_path_and_query() {
        assert_eq!(
            redact_endpoint("http://localhost:11434/api/chat"),
            "localhost:11434"
        );
    }

    #[test]
    fn redact_endpoint_strips_credentials() {
        assert_eq!(
            redact_endpoint("https://user:pass@example.com/foo"),
            "example.com"
        );
    }

    #[test]
    fn redact_endpoint_no_scheme_passthrough() {
        // No "://" present — treat the whole string as host/path; still strip path.
        assert_eq!(redact_endpoint("localhost:11434/api"), "localhost:11434");
    }

    #[test]
    fn redact_endpoint_just_host() {
        assert_eq!(redact_endpoint("https://example.com"), "example.com");
    }

    #[test]
    fn redact_endpoint_strips_fragment() {
        assert_eq!(redact_endpoint("http://host:9090/path#frag"), "host:9090");
    }

    #[test]
    fn redact_endpoint_strips_query() {
        assert_eq!(redact_endpoint("http://host/path?q=1"), "host");
    }

    #[test]
    fn redact_endpoint_empty_does_not_panic() {
        let r = redact_endpoint("");
        // Empty input: no scheme, no host — returns empty string.
        assert_eq!(r, "");
    }

    #[test]
    fn redact_endpoint_ollama_style() {
        assert_eq!(
            redact_endpoint("http://127.0.0.1:11434/v1/chat/completions"),
            "127.0.0.1:11434"
        );
    }
}
