use super::*;

#[test]
fn test_scrub_credentials_utf8() {
    // Regex requires at least 8 chars for the value
    // The [a-zA-Z0-9_\-\.]{8,} part of the regex does NOT match emoji
    // So we must use quotes to hit the "([^"]{8,})" part
    let input = "api_key: \"🦀🦀🦀🦀🦀🦀🦀🦀\"";
    let output = scrub_credentials(input);
    // Should preserve 4 crabs and then redact
    assert!(output.contains("🦀🦀🦀🦀*[REDACTED]"));
}

#[test]
fn test_scrub_credentials_short_val() {
    let input = "api_key: 12345678";
    let output = scrub_credentials(input);
    assert!(output.contains("api_key: 1234*[REDACTED]"));
}

// #4453: bare, unlabelled secrets that show up in env dumps / API responses.

#[test]
fn scrubs_bare_aws_access_key() {
    let out = scrub_credentials("config dump AKIAIOSFODNN7EXAMPLE trailing text");
    assert!(
        !out.contains("AKIAIOSFODNN7EXAMPLE"),
        "bare AWS access key must be redacted: {out}"
    );
    assert!(
        out.contains("[REDACTED]"),
        "redaction marker present: {out}"
    );
}

#[test]
fn scrubs_bare_openai_key() {
    let out = scrub_credentials("response body sk-abcdefghij1234567890ABCDEFGHIJ end");
    assert!(
        !out.contains("abcdefghij1234567890ABCDEFGHIJ"),
        "openai secret body must be redacted: {out}"
    );
    assert!(
        out.contains("sk-"),
        "the sk- scheme is kept for context: {out}"
    );
    assert!(
        out.contains("[REDACTED]"),
        "redaction marker present: {out}"
    );
}

#[test]
fn scrubs_space_separated_bearer_token() {
    let out = scrub_credentials("Authorization: Bearer abcDEF1234567890ghijklmnop done");
    assert!(
        !out.contains("abcDEF1234567890ghijklmnop"),
        "space-separated bearer token must be redacted: {out}"
    );
    assert!(
        out.contains("Bearer"),
        "the scheme word is kept for context: {out}"
    );
    assert!(
        out.contains("[REDACTED]"),
        "redaction marker present: {out}"
    );
}
