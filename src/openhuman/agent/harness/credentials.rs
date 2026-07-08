use regex::Regex;
use std::sync::LazyLock;

/// Key/value credential shapes: `token: "…"`, `api_key=…`, `bearer: …`, etc.
static SENSITIVE_KV_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(token|api[_-]?key|password|secret|user[_-]?key|bearer|credential)["']?\s*[:=]\s*(?:"([^"]{8,})"|'([^']{8,})'|([a-zA-Z0-9_\-\.]{8,}))"#).unwrap()
});

/// Bare AWS access-key IDs — `AKIA…`/`ASIA…` followed by 16 base32 chars — which
/// appear naked in env dumps and config reads with no surrounding key name.
static AWS_ACCESS_KEY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b((?:AKIA|ASIA)[0-9A-Z]{16})\b").unwrap());

/// Bare OpenAI-style secret keys — `sk-…` (incl. `sk-proj-…`) with a long token
/// body. Not necessarily attached to a `key:` label in raw API responses.
static OPENAI_KEY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(sk-[A-Za-z0-9_\-]{16,})\b").unwrap());

/// Space-separated bearer tokens as they appear in HTTP auth headers
/// (`Authorization: Bearer <token>`) — the KV regex only catches `bearer:`/`=`.
static BEARER_SPACE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(Bearer)\s+([A-Za-z0-9_\-\.=+/]{16,})").unwrap());

/// Preserve the first 4 chars of `val` for context, returning the redacted
/// prefix (empty when the value is too short to safely reveal any of it).
fn redact_prefix(val: &str) -> &str {
    if val.chars().count() > 4 {
        match val.char_indices().nth(4) {
            Some((idx, _)) => &val[..idx],
            None => val,
        }
    } else {
        ""
    }
}

/// Scrub credentials from tool output to prevent accidental exfiltration.
/// Replaces known credential patterns with a redacted placeholder while preserving
/// a small prefix for context.
///
/// Covers labelled key/value pairs plus bare secrets that show up unlabelled in
/// env dumps, config reads and API responses: AWS access-key IDs (`AKIA…`/
/// `ASIA…`), OpenAI-style `sk-…` keys, and space-separated `Bearer <token>`
/// auth headers.
pub(crate) fn scrub_credentials(input: &str) -> String {
    let stage_kv = SENSITIVE_KV_REGEX.replace_all(input, |caps: &regex::Captures| {
        let full_match = &caps[0];
        let key = &caps[1];
        let val = caps
            .get(2)
            .or(caps.get(3))
            .or(caps.get(4))
            .map(|m| m.as_str())
            .unwrap_or("");

        let prefix = redact_prefix(val);

        if full_match.contains(':') {
            if full_match.contains('"') {
                format!("\"{}\": \"{}*[REDACTED]\"", key, prefix)
            } else {
                format!("{}: {}*[REDACTED]", key, prefix)
            }
        } else if full_match.contains('=') {
            if full_match.contains('"') {
                format!("{}=\"{}*[REDACTED]\"", key, prefix)
            } else {
                format!("{}={}*[REDACTED]", key, prefix)
            }
        } else {
            format!("{}: {}*[REDACTED]", key, prefix)
        }
    });

    // Bare AWS access-key IDs: keep the 4-char `AKIA`/`ASIA` prefix for context.
    let stage_aws = AWS_ACCESS_KEY_REGEX.replace_all(&stage_kv, |caps: &regex::Captures| {
        format!("{}*[REDACTED]", redact_prefix(&caps[1]))
    });

    // Bare `sk-…` keys: keep the `sk-` scheme, redact the secret body.
    let stage_openai = OPENAI_KEY_REGEX.replace_all(&stage_aws, |_caps: &regex::Captures| {
        "sk-*[REDACTED]".to_string()
    });

    // Space-separated `Bearer <token>`: keep the scheme word, redact the token.
    BEARER_SPACE_REGEX
        .replace_all(&stage_openai, |caps: &regex::Captures| {
            format!("{} *[REDACTED]", &caps[1])
        })
        .to_string()
}

#[cfg(test)]
mod tests {
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
}
