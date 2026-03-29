//! Parse model output into suggestions and inline completions.

use super::types::Suggestion;

pub(crate) fn parse_suggestions(raw: &str, limit: usize) -> Vec<Suggestion> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(limit)
        .map(|text| Suggestion {
            text: text.to_string(),
            confidence: 0.65,
        })
        .collect()
}

pub(crate) fn sanitize_inline_completion(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return String::new();
    }

    let mut cleaned = line
        .trim_matches('"')
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '>' | '1'..='9' | '.' | ')'))
        .trim()
        .replace('\t', " ");

    if cleaned.eq_ignore_ascii_case("none") || cleaned.eq_ignore_ascii_case("n/a") {
        return String::new();
    }

    if cleaned.chars().count() > 128 {
        cleaned = cleaned.chars().take(128).collect();
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_suggestions_strips_numbering_and_respects_limit() {
        let raw = "1. First idea\n- Second idea\n3) Third idea\n";
        let out = parse_suggestions(raw, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "First idea");
        assert_eq!(out[1].text, "Second idea");
        assert!((out[0].confidence - 0.65).abs() < f32::EPSILON);
    }

    #[test]
    fn sanitize_inline_completion_handles_placeholders_and_clamps_length() {
        assert_eq!(sanitize_inline_completion("none"), "");
        assert_eq!(sanitize_inline_completion("n/a"), "");
        assert_eq!(
            sanitize_inline_completion("\"- hello world\""),
            "hello world"
        );

        let long = "a".repeat(256);
        let out = sanitize_inline_completion(&long);
        assert_eq!(out.chars().count(), 128);
    }
}
