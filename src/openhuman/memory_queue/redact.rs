//! Log-side scrubber for free-form `reason` / `last_error` strings emitted
//! by [`worker::run_once`] (defer/fail branches) and the matching
//! [`store::mark_failed`] / [`store::mark_deferred`] log lines.
//!
//! Persisted DB state (`mem_tree_jobs.last_error`) keeps the full
//! original string for diagnostics — handlers may attach
//! upstream-provider responses or full anyhow chains there. Logs are a
//! lower-trust sink (often forwarded to remote log aggregators or
//! shared in bug reports), so we apply a uniform scrub policy to:
//!
//! 1. Mask credential-shaped tokens (Bearer, OpenAI `sk-…`, GitHub
//!    `ghp_…`, Slack `xox?-…`, generic `api_key=…` / `password=…` /
//!    `token=…` assignments).
//! 2. Strip URL userinfo (`https://user:pass@host` → `https://***@host`).
//! 3. Mask bare email addresses.
//! 4. Cap the logged string at [`MAX_LEN`] bytes, suffixed with
//!    `…(truncated, N more bytes)` so a reader knows the original
//!    string was longer.
//!
//! [`worker::run_once`]: super::worker::run_once

use std::sync::OnceLock;

use regex::Regex;

/// Upper bound on the byte length of a scrubbed string. Long enough to
/// keep the head of an anyhow `{:#}` chain (which usually includes the
/// most useful context) but short enough that one bad job can't flood
/// the log with megabytes of provider response body.
pub(crate) const MAX_LEN: usize = 1024;

/// Scrub a free-form error / reason string for emission to logs.
/// Returns an owned `String` because every regex pass may rewrite the
/// input; callers are emitting this through `format!` / `log::*!`
/// anyway, so the allocation isn't on a hot path.
pub(crate) fn scrub_for_log(input: &str) -> String {
    let mut out = input.to_owned();
    for (re, replacement) in patterns() {
        // `Cow::into_owned` is cheap when the regex didn't match.
        out = re.replace_all(&out, *replacement).into_owned();
    }
    truncate(out)
}

fn truncate(mut s: String) -> String {
    if s.len() <= MAX_LEN {
        return s;
    }
    // Round down to a char boundary to avoid splitting a multi-byte
    // UTF-8 sequence — `truncate` itself panics on a non-boundary.
    let mut cut = MAX_LEN;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let dropped = s.len() - cut;
    s.truncate(cut);
    s.push_str(&format!("…(truncated, {dropped} more bytes)"));
    s
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // URL userinfo: capture the scheme + ://, then the userinfo
            // up to '@'. Replace with ***@. Anchored on `://` to avoid
            // eating bare `user@host` strings — those are handled by
            // the email rule below.
            (
                Regex::new(r"(?P<scheme>[a-zA-Z][a-zA-Z0-9+.\-]*://)[^\s/@]+@").unwrap(),
                "$scheme***@",
            ),
            // Bearer tokens (with or without an `Authorization:` prefix).
            (
                Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-+/=]+").unwrap(),
                "Bearer ***",
            ),
            // Provider-prefixed credentials with stable, well-known
            // shapes. Listed individually so a future reader can see
            // exactly which providers are covered.
            (
                Regex::new(r"sk-[A-Za-z0-9_\-]{16,}").unwrap(),
                "sk-***",
            ),
            (
                Regex::new(r"ghp_[A-Za-z0-9]{20,}").unwrap(),
                "ghp_***",
            ),
            (
                Regex::new(r"ghs_[A-Za-z0-9]{20,}").unwrap(),
                "ghs_***",
            ),
            (
                Regex::new(r"gho_[A-Za-z0-9]{20,}").unwrap(),
                "gho_***",
            ),
            (
                Regex::new(r"xox[abprs]-[A-Za-z0-9\-]{8,}").unwrap(),
                "xox-***",
            ),
            // Generic `key=value` assignments where the key name implies
            // a secret. Matches `api_key`, `apiKey`, `api-key`,
            // `password`, `passwd`, `pwd`, `token`, `secret`. Accepts
            // a quoted, single-quoted, or bare value; bare values stop
            // at the first whitespace / comma / closing bracket so we
            // don't eat the rest of the message.
            (
                Regex::new(
                    r#"(?i)(?P<k>api[_\-]?key|password|passwd|pwd|secret|token|auth)\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s,}\)\]]+)"#,
                )
                .unwrap(),
                "$k=***",
            ),
            // Bare email addresses. Conservative pattern (no UTF-8
            // local parts, no quoted local parts) — sufficient to mask
            // the common cases without eating unrelated `@` symbols.
            (
                Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap(),
                "***@***",
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_for_safe_string() {
        let input = "rate_limited: provider returned 429, retry in 30s";
        assert_eq!(scrub_for_log(input), input);
    }

    #[test]
    fn masks_bearer_token() {
        let s = scrub_for_log("Authorization: Bearer abc123.def-456_xyz");
        assert!(s.contains("Bearer ***"), "got {s:?}");
        assert!(!s.contains("abc123"));
    }

    #[test]
    fn masks_openai_key() {
        let s = scrub_for_log("upstream returned 401: invalid sk-abcDEF1234567890ZZZZ key");
        assert!(s.contains("sk-***"));
        assert!(!s.contains("sk-abcDEF1234567890ZZZZ"));
    }

    #[test]
    fn masks_github_token_variants() {
        for raw in [
            "ghp_abcdefghij1234567890ABCD",
            "ghs_abcdefghij1234567890ABCD",
            "gho_abcdefghij1234567890ABCD",
        ] {
            let s = scrub_for_log(&format!("error: token {raw} rejected"));
            assert!(s.contains("***"), "input={raw} out={s}");
            assert!(!s.contains(raw), "input={raw} out={s}");
        }
    }

    #[test]
    fn masks_slack_token() {
        let s = scrub_for_log("posting to slack failed: xoxb-1234567890-abcdEFG");
        assert!(s.contains("xox-***"));
        assert!(!s.contains("xoxb-1234567890"));
    }

    #[test]
    fn masks_generic_secret_assignments() {
        let inputs = [
            ("api_key=hunter2 trailing", "api_key=***"),
            ("password: hunter2", "password=***"),
            ("Token = hunter2,more", "Token=***"),
            ("apiKey=\"hunter2\"", "apiKey=***"),
        ];
        for (raw, expect) in inputs {
            let s = scrub_for_log(raw);
            assert!(s.contains(expect), "input={raw:?} out={s:?}");
            assert!(!s.contains("hunter2"), "input={raw:?} out={s:?}");
        }
    }

    #[test]
    fn strips_url_userinfo() {
        let s = scrub_for_log("connect failed https://alice:s3cret@db.internal/x");
        assert!(s.contains("https://***@db.internal/x"), "got {s:?}");
        assert!(!s.contains("alice"));
        assert!(!s.contains("s3cret"));
    }

    #[test]
    fn masks_email() {
        let s = scrub_for_log("user alice@example.com triggered job");
        assert!(s.contains("***@***"), "got {s:?}");
        assert!(!s.contains("alice"));
        assert!(!s.contains("example.com"));
    }

    #[test]
    fn truncates_oversized_input() {
        let big = "x".repeat(MAX_LEN * 2);
        let s = scrub_for_log(&big);
        assert!(s.len() < big.len());
        assert!(s.contains("(truncated,"));
    }

    #[test]
    fn truncate_handles_multibyte_boundary() {
        // `é` is 2 bytes in UTF-8; build a string whose naïve cut at
        // MAX_LEN would land mid-codepoint.
        let mut big = "a".repeat(MAX_LEN - 1);
        big.push('é');
        big.push_str(&"b".repeat(64));
        let s = scrub_for_log(&big);
        // Must not panic; must produce valid UTF-8 (String guarantees).
        assert!(s.contains("(truncated,"));
    }

    #[test]
    fn idempotent_on_already_scrubbed_string() {
        let once = scrub_for_log("Bearer abcdef api_key=hunter2");
        let twice = scrub_for_log(&once);
        assert_eq!(once, twice);
    }
}
