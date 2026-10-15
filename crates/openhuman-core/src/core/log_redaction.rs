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

/// Input above this size is suppressed rather than scanned or printed (ADR 7).
pub const FLOOR_MAX_INPUT_BYTES: usize = 256 * 1024;

/// Replacement for suppressed (oversized) input.
pub const SUPPRESSED: &str = "[REDACTED:suppressed]";

static SECRET_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // URL userinfo (`scheme://user:pass@host`); runs first so the query rule
        // below sees the stripped authority.
        (
            Regex::new(r"(?i)(\b[a-z][a-z0-9+.\-]*://)[^/\s:@]+(?::[^/\s@]*)?@").unwrap(),
            "${1}[REDACTED]@",
        ),
        // URL query strings and fragments.
        (
            Regex::new(r"(?i)(\b[a-z][a-z0-9+.\-]*://[^\s?#]+)[?#]\S*").unwrap(),
            "${1}?[REDACTED]",
        ),
        // `Authorization: Basic/Digest <credentials>`.
        (
            Regex::new(r"(?i)(\b(?:basic|digest)\s+)[A-Za-z0-9+/=._\-]{8,}").unwrap(),
            "${1}[REDACTED]",
        ),
        // `Cookie:` / `Set-Cookie:` header lines (whole value).
        (
            Regex::new(r"(?i)\b((?:set-)?cookie\s*:\s*)[^\r\n]+").unwrap(),
            "${1}[REDACTED]",
        ),
        // password / secret assignments, quoted or bare.
        (
            Regex::new(
                r#"(?i)\b((?:password|passwd|client[_-]?secret|secret)["']?\s*[=:]\s*["']?)[^\s,;&"']+"#,
            )
            .unwrap(),
            "${1}[REDACTED]",
        ),
        // Native home-directory prefixes.
        (
            Regex::new(r"(/(?:home|Users)/)[^/\s]+").unwrap(),
            "${1}[REDACTED]",
        ),
        (
            Regex::new(r"(?i)([A-Za-z]:\\Users\\)[^\\\s]+").unwrap(),
            "${1}[REDACTED]",
        ),
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
    scrub_with(input, MODULE_SCRUBBER.get().copied())
}

/// A synchronous view of the TinySecurity `Redact` module. It receives text the
/// floor already redacted and returns the module's stricter answer, or `None`
/// when the module cannot answer in time (the floor result then stands).
pub type ModuleScrubber = fn(&str) -> Option<String>;

static MODULE_SCRUBBER: std::sync::OnceLock<ModuleScrubber> = std::sync::OnceLock::new();

/// Arm the module-backed second pass once the module is ready. Idempotent; the
/// first installer wins.
pub fn install_module_scrubber(scrubber: ModuleScrubber) {
    if MODULE_SCRUBBER.set(scrubber).is_ok() {
        log::debug!("[security][floor] module scrubber armed");
    }
}

/// The floor, then the module pass when one is supplied. The module only ever
/// sees floor output, and a module that cannot answer leaves the floor result,
/// never the original text.
pub(crate) fn scrub_with(input: &str, module: Option<ModuleScrubber>) -> String {
    let floored = floor_scrub(input);
    if floored == SUPPRESSED {
        return floored;
    }
    match module.and_then(|scrub| scrub(&floored)) {
        Some(stronger) => stronger,
        None => floored,
    }
}

fn floor_scrub(input: &str) -> String {
    if input.len() > FLOOR_MAX_INPUT_BYTES {
        log::debug!(
            "[security][floor] suppressed oversized input bytes={}",
            input.len()
        );
        return SUPPRESSED.to_string();
    }
    let mut result = input.to_string();
    for (re, replacement) in SECRET_PATTERNS.iter() {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    result
}

#[cfg(test)]
#[path = "log_redaction_tests.rs"]
mod tests;
