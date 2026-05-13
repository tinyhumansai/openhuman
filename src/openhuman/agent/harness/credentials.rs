use regex::Regex;
use std::sync::LazyLock;

static SENSITIVE_KV_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(token|api[_-]?key|password|secret|user[_-]?key|bearer|credential)["']?\s*[:=]\s*(?:"([^"]{8,})"|'([^']{8,})'|([a-zA-Z0-9_\-\.]{8,}))"#).unwrap()
});

/// Scrub credentials from tool output to prevent accidental exfiltration.
/// Replaces known credential patterns with a redacted placeholder while preserving
/// a small prefix for context.
pub(crate) fn scrub_credentials(input: &str) -> String {
    SENSITIVE_KV_REGEX
        .replace_all(input, |caps: &regex::Captures| {
            let full_match = &caps[0];
            let key = &caps[1];
            let val = caps
                .get(2)
                .or(caps.get(3))
                .or(caps.get(4))
                .map(|m| m.as_str())
                .unwrap_or("");

            // Preserve first 4 chars for context, then redact
            let prefix = if val.chars().count() > 4 {
                match val.char_indices().nth(4) {
                    Some((idx, _)) => &val[..idx],
                    None => val,
                }
            } else {
                ""
            };

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
}
