//! ADR 0007's fixed boot-floor redactor for diagnostics emitted before
//! TinySecurity is ready. Once the module loads, both Sentry and log callers
//! also use its shared redactor through the installed callback.

use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::OnceLock;

/// Fixed boot floor used before the TinySecurity redactor is ready.
pub(crate) const SUPPRESSED: &str = "[REDACTED]";

/// The TinySecurity pass is installed after its released module is serving.
static MODULE_SCRUBBER: OnceLock<fn(&str) -> Option<String>> = OnceLock::new();

/// Install the module-backed second pass once TinySecurity has loaded.
pub(crate) fn install_module_scrubber(scrubber: fn(&str) -> Option<String>) {
    let _ = MODULE_SCRUBBER.set(scrubber);
}

fn apply_module_scrubber(
    floor_result: &str,
    scrubber: Option<fn(&str) -> Option<String>>,
) -> String {
    scrubber
        .and_then(|scrub| scrub(floor_result))
        .unwrap_or_else(|| floor_result.to_owned())
}

static SECRET_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        // Matches "Bearer <token>" and redacts the token.
        (Regex::new(r"(?i)(bearer\s+)\S+").unwrap(), "${1}[REDACTED]"),
        // An `Authorization: Basic …` header, however short its credential.
        (
            Regex::new(r#"(?i)(authorization["']?\s*[:=]\s*["']?basic\s+)[A-Za-z0-9+/]+=*"#)
                .unwrap(),
            "${1}[REDACTED]",
        ),
        // HTTP Basic credentials.
        (
            Regex::new(r"(?i)(basic\s+)[A-Za-z0-9+/=]{8,}").unwrap(),
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

/// A credential key: `token` with any snake_case prefix (`bot_token`,
/// `access_token`), `api_key`, `client_secret` or `password`.
const CREDENTIAL_KEY: &str = r"(?i)\b((?:[a-z0-9]+_)*token|api[_-]?key|client_secret|password)";

/// Named credential keys and their values, most specific first. `cancellation_token`
/// matches the key shape but is not a secret, so `scrub_secrets` leaves it alone.
static NAMED_KEY_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    [
        // A quoted value: `"password":"a b, c"`. It runs to its closing quote
        // (escapes included), so spaces and delimiters inside can't end it early.
        (r#"("?\s*[=:]\s*")(?:[^"\\]|\\.)*""#, "${1}${2}[REDACTED]\""),
        // The same inside escaped JSON: `{\"token\":\"x y\"}`.
        (
            r#"(\\"\s*:\s*\\")(?:[^"\\]|\\[^"])*\\""#,
            "${1}${2}[REDACTED]\\\"",
        ),
        // An unquoted value: `api_key=…`, `access_token: …`.
        (
            r#"(\\?"?\s*[=:]\s*\\?"?)(?:[^\s"\\&,;}]|\\[^"])+"#,
            "${1}${2}[REDACTED]",
        ),
    ]
    .into_iter()
    .map(|(value, replacement)| {
        (
            Regex::new(&format!("{CREDENTIAL_KEY}{value}")).unwrap(),
            replacement,
        )
    })
    .collect()
});

/// Replace substrings that look like secrets with `[REDACTED]`.
///
/// Intended for anything about to be written to a log sink or an error report;
/// it redacts the secret-looking span in place and leaves the rest of the
/// diagnostic message intact (unlike a whole-value prefix redaction).
pub fn scrub_secrets(input: &str) -> String {
    let mut result = input.to_string();
    for (re, replacement) in NAMED_KEY_PATTERNS.iter() {
        result = re
            .replace_all(&result, |caps: &regex::Captures<'_>| {
                if caps[1].eq_ignore_ascii_case("cancellation_token") {
                    caps[0].to_string()
                } else {
                    let mut out = String::new();
                    caps.expand(replacement, &mut out);
                    out
                }
            })
            .into_owned();
    }
    for (re, replacement) in SECRET_PATTERNS.iter() {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    apply_module_scrubber(&result, MODULE_SCRUBBER.get().copied())
}

#[cfg(test)]
#[path = "log_redaction_tests.rs"]
mod tests;
