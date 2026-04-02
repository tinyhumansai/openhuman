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
