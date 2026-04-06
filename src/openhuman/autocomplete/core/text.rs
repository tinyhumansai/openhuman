//! Text utilities for autocomplete suggestions.

use super::types::MAX_SUGGESTION_CHARS;

pub(super) use crate::openhuman::accessibility::truncate_tail;

/// Truncate to the first `max_chars` characters (preserves the start of the string).
pub(super) fn truncate_head(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(super) fn sanitize_suggestion(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    let cleaned = first_line
        .trim_matches('"')
        .replace('\t', " ")
        .replace('\r', "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    truncate_head(&cleaned, MAX_SUGGESTION_CHARS)
}

pub(super) fn is_no_text_candidate_error(err: &str) -> bool {
    err.contains("ERROR:no_text_candidate_found")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- truncate_head ---

    #[test]
    fn truncate_head_shorter_than_max_returns_original() {
        assert_eq!(truncate_head("hello", 10), "hello");
    }

    #[test]
    fn truncate_head_exactly_max_returns_original() {
        assert_eq!(truncate_head("hello", 5), "hello");
    }

    #[test]
    fn truncate_head_longer_than_max_returns_head() {
        assert_eq!(truncate_head("hello world", 5), "hello");
    }

    #[test]
    fn truncate_head_empty_string() {
        assert_eq!(truncate_head("", 5), "");
    }

    #[test]
    fn truncate_head_zero_max_returns_empty() {
        assert_eq!(truncate_head("hello", 0), "");
    }

    #[test]
    fn truncate_head_multibyte_chars_counts_codepoints() {
        // "héllo" is 5 chars; first 3 = "hél"
        assert_eq!(truncate_head("héllo", 3), "hél");
    }

    // --- sanitize_suggestion ---

    #[test]
    fn sanitize_suggestion_plain_text() {
        assert_eq!(sanitize_suggestion("hello world"), "hello world");
    }

    #[test]
    fn sanitize_suggestion_trims_leading_and_trailing_whitespace() {
        assert_eq!(sanitize_suggestion("  hello  "), "hello");
    }

    #[test]
    fn sanitize_suggestion_strips_surrounding_double_quotes() {
        assert_eq!(sanitize_suggestion("\"quoted\""), "quoted");
    }

    #[test]
    fn sanitize_suggestion_takes_first_line_only() {
        assert_eq!(sanitize_suggestion("line one\nline two"), "line one");
    }

    #[test]
    fn sanitize_suggestion_crlf_newline_takes_first_line() {
        assert_eq!(sanitize_suggestion("line one\r\nline two"), "line one");
    }

    #[test]
    fn sanitize_suggestion_replaces_embedded_tabs_with_spaces() {
        // Leading/trailing tabs are stripped by trim(); only interior tabs become spaces.
        assert_eq!(sanitize_suggestion("he\tllo"), "he llo");
    }

    #[test]
    fn sanitize_suggestion_empty_input_returns_empty() {
        assert_eq!(sanitize_suggestion(""), "");
    }

    #[test]
    fn sanitize_suggestion_whitespace_only_returns_empty() {
        assert_eq!(sanitize_suggestion("   \n   "), "");
    }

    #[test]
    fn sanitize_suggestion_truncates_to_max_chars() {
        // MAX_SUGGESTION_CHARS is 64 — a 70-char string should be cut to 64.
        let long = "a".repeat(70);
        let result = sanitize_suggestion(&long);
        assert_eq!(result.len(), 64);
        assert!(result.chars().all(|c| c == 'a'));
    }

    #[test]
    fn sanitize_suggestion_exactly_max_chars_unchanged() {
        let exact = "b".repeat(64);
        assert_eq!(sanitize_suggestion(&exact), exact);
    }

    #[test]
    fn sanitize_suggestion_removes_bare_carriage_return() {
        // Bare \r is NOT treated as a line ending by lines(), so it stays in the
        // first-line content and is then removed by replace('\r', "").
        assert_eq!(sanitize_suggestion("hello\rworld"), "helloworld");
    }

    // --- is_no_text_candidate_error ---

    #[test]
    fn is_no_text_candidate_error_exact_match() {
        assert!(is_no_text_candidate_error("ERROR:no_text_candidate_found"));
    }

    #[test]
    fn is_no_text_candidate_error_substring_match() {
        assert!(is_no_text_candidate_error(
            "AX query failed: ERROR:no_text_candidate_found"
        ));
    }

    #[test]
    fn is_no_text_candidate_error_unrelated_error() {
        assert!(!is_no_text_candidate_error("some other error"));
    }

    #[test]
    fn is_no_text_candidate_error_empty_string() {
        assert!(!is_no_text_candidate_error(""));
    }

    #[test]
    fn is_no_text_candidate_error_partial_prefix_no_match() {
        assert!(!is_no_text_candidate_error("ERROR:no_text"));
    }
}
