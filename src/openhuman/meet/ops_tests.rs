//! Additional unit tests for `meet::ops` covering edge cases not exercised
//! by the inline `#[cfg(test)] mod tests` block in `ops.rs`.
//!
//! These tests are colocated as `ops_tests.rs` and wired in via
//! `#[cfg(test)] #[path = "ops_tests.rs"] mod ops_tests;` in `ops.rs`.

use super::*;

// ── validate_meet_url — URL normalization ─────────────────────────────────────

/// A URL with UTM / authuser tracking parameters should still be accepted
/// because the path remains a valid Meet code. The returned `url::Url` retains
/// the query string — callers that forward the URL to the shell should use
/// `normalized_url.as_str()`.
#[test]
fn accepts_meet_code_url_with_query_params() {
    let url_with_params = "https://meet.google.com/abc-defg-hij?authuser=0&utm_source=calendar";
    let u = validate_meet_url(url_with_params).unwrap();
    // Host must still be meet.google.com
    assert_eq!(u.host_str(), Some("meet.google.com"));
    // Path must still be the meet code
    assert_eq!(u.path(), "/abc-defg-hij");
}

/// A URL with a fragment should also be accepted; only scheme and host/path
/// are validated.
#[test]
fn accepts_meet_code_url_with_fragment() {
    let u = validate_meet_url("https://meet.google.com/abc-defg-hij#top").unwrap();
    assert_eq!(u.host_str(), Some("meet.google.com"));
    assert_eq!(u.path(), "/abc-defg-hij");
}

/// Leading / trailing whitespace in the raw URL is trimmed before parsing.
#[test]
fn trims_whitespace_from_raw_url() {
    validate_meet_url("  https://meet.google.com/abc-defg-hij  ").unwrap();
}

/// Meet codes must have all-lowercase alpha segments. Upper-case are rejected.
#[test]
fn rejects_uppercase_meet_code_segments() {
    // All three segments must be all-lowercase ASCII.
    assert!(validate_meet_url("https://meet.google.com/ABC-defg-hij").is_err());
    assert!(validate_meet_url("https://meet.google.com/abc-DEFG-hij").is_err());
}

/// Meet code segments must each be at least 3 characters long.
#[test]
fn rejects_too_short_meet_code_segments() {
    // Each part needs len >= 3
    assert!(validate_meet_url("https://meet.google.com/ab-defg-hij").is_err());
    assert!(validate_meet_url("https://meet.google.com/abc-de-hij").is_err());
    assert!(validate_meet_url("https://meet.google.com/abc-defg-hi").is_err());
}

/// A code with numeric digits is rejected — only ascii lowercase letters are
/// valid in a meet code segment.
#[test]
fn rejects_meet_code_with_digits() {
    assert!(validate_meet_url("https://meet.google.com/abc-def1-hij").is_err());
}

/// `lookup/<id>` with a query string should still be accepted.
#[test]
fn accepts_lookup_url_with_query_params() {
    validate_meet_url("https://meet.google.com/lookup/abcdef1234?authuser=1").unwrap();
}

/// `lookup/` alone (empty id) must be rejected.
#[test]
fn rejects_lookup_with_empty_id() {
    assert!(validate_meet_url("https://meet.google.com/lookup/").is_err());
}

// ── validate_display_name — boundary and edge cases ───────────────────────────

/// Exactly 64 characters is the limit and must be accepted.
#[test]
fn accepts_display_name_at_64_chars() {
    let name_64 = "a".repeat(64);
    let result = validate_display_name(&name_64).unwrap();
    assert_eq!(result.len(), 64);
}

/// 65 characters must be rejected.
#[test]
fn rejects_display_name_at_65_chars() {
    assert!(validate_display_name(&"a".repeat(65)).is_err());
}

/// A name consisting only of whitespace is treated as empty after trimming.
#[test]
fn rejects_whitespace_only_display_name() {
    assert!(validate_display_name("\t \n").is_err());
}

/// Other control characters (non-newline) are also rejected.
#[test]
fn rejects_display_name_with_control_chars() {
    assert!(validate_display_name("hello\x01world").is_err());
    assert!(validate_display_name("name\x08").is_err()); // backspace
}

/// A single printable character is the minimum non-empty name.
#[test]
fn accepts_single_character_display_name() {
    assert_eq!(validate_display_name("A").unwrap(), "A");
}

/// Non-ASCII unicode (e.g. Japanese) is allowed as long as the *character*
/// count is ≤ 64 — the validator counts chars, not bytes.
#[test]
fn accepts_unicode_display_name_within_char_limit() {
    // Each Japanese character is 3 bytes but counts as 1 char.
    let unicode_name = "日本語テスト"; // 6 chars
    let result = validate_display_name(unicode_name).unwrap();
    assert_eq!(result, unicode_name);
}

/// Unicode name with exactly 64 characters should pass.
#[test]
fn accepts_unicode_display_name_at_64_chars() {
    // 64 x '日' = 64 chars, 192 bytes — still within char budget.
    let name = "日".repeat(64);
    validate_display_name(&name).unwrap();
}

/// 65 unicode characters must be rejected even if byte count is different.
#[test]
fn rejects_unicode_display_name_at_65_chars() {
    let name = "日".repeat(65);
    assert!(validate_display_name(&name).is_err());
}
