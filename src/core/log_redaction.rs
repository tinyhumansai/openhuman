//! Shared secret-scrubbing for anything written to stderr / file logs.
//!
//! Diagnostic log lines (e.g. `core::observability::report_error_message`) can
//! carry error strings that embed bearer tokens, API keys, or other secrets. In
//! slim builds compiled without `crash-reporting` there is no Sentry
//! `before_send` hook to sanitise them, and even in full builds the
//! `before_send` hook only scrubs the *Sentry event* — not the parallel
//! `tracing` log line. This module owns the one redaction pass used by both the
//! Sentry path (`src/main.rs`) and the always-on log path, so the patterns
//! cannot drift between them. Always compiled (no feature gate).

use once_cell::sync::Lazy;
use regex::Regex;

static SECRET_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // Matches "Bearer <token>" and redacts the token.
        (Regex::new(r"(?i)(bearer\s+)\S+").unwrap(), "${1}[REDACTED]"),
        // Matches "api-key: <key>" or "api_key=<key>" and redacts the key.
        (
            Regex::new(r"(?i)(api[_-]?key[=:\s]+)\S+").unwrap(),
            "${1}[REDACTED]",
        ),
        // \b anchor prevents matching `cancellation_token=` etc.
        (
            Regex::new(r"(?i)\b(token[=:\s]+)\S+").unwrap(),
            "${1}[REDACTED]",
        ),
        // Anthropic keys (sk-ant-api03-...) contain hyphens the generic
        // sk- pattern below won't match.
        (
            Regex::new(r"sk-ant-[A-Za-z0-9\-_]{16,}").unwrap(),
            "[REDACTED]",
        ),
        // OpenAI admin keys (sk-admin-...).
        (
            Regex::new(r"sk-admin-[A-Za-z0-9\-_]{12,}").unwrap(),
            "[REDACTED]",
        ),
        // OpenAI project-scoped and org-scoped keys (sk-proj-... / sk-org-...).
        (
            Regex::new(r"sk-(?:proj|org)-[A-Za-z0-9\-_]{12,}").unwrap(),
            "[REDACTED]",
        ),
        // Generic catch-all for any sk- format not covered above. Includes `-`
        // and `_` in the suffix so a separator mid-token can't leave a trailing
        // fragment unredacted (e.g. `sk-…_uv` → `[REDACTED]_uv`).
        (Regex::new(r"sk-[A-Za-z0-9_-]{20,}").unwrap(), "[REDACTED]"),
    ]
});

/// Replace substrings that look like secrets with `[REDACTED]`.
///
/// Intended for anything about to be written to a log sink or an error report;
/// it redacts the secret-looking span in place and leaves the rest of the
/// diagnostic message intact (unlike a whole-value prefix redaction).
pub fn scrub_secrets(input: &str) -> String {
    let mut result = input.to_string();
    for (re, replacement) in SECRET_PATTERNS.iter() {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::scrub_secrets;

    #[test]
    fn scrubs_bearer_token() {
        assert_eq!(
            scrub_secrets("Authorization: Bearer abc123xyz"),
            "Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn scrubs_api_key_assignment() {
        assert_eq!(scrub_secrets("api_key=sk-abc123"), "api_key=[REDACTED]");
    }

    #[test]
    fn scrubs_anthropic_key() {
        assert_eq!(
            scrub_secrets("key: sk-ant-api03-abcdefghijklmnop"),
            "key: [REDACTED]"
        );
    }

    #[test]
    fn scrubs_bare_generic_sk_key() {
        assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrstuvwx"), "[REDACTED]");
    }

    #[test]
    fn scrubs_generic_sk_key_with_separators() {
        // A `_` or `-` mid-suffix must not leave a trailing fragment unredacted.
        assert_eq!(scrub_secrets("sk-abcdefghijklmnopqrst_uv"), "[REDACTED]");
        assert_eq!(scrub_secrets("sk-abcdefghij-klmnopqrst_uv"), "[REDACTED]");
    }

    #[test]
    fn leaves_plain_diagnostics_intact() {
        let msg = "profile 42: derived rate clamp exceeded (max_iterations=8)";
        assert_eq!(scrub_secrets(msg), msg);
    }
}
