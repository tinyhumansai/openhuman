//! Pure helpers for generating and validating conversation thread titles.
//!
//! Extracted from `threads::ops` so the parsing / sanitisation rules can be
//! unit-tested without pulling in `Config`, provider runtime, or RPC wiring.

use std::hash::{Hash, Hasher};

pub const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
pub const THREAD_TITLE_MODEL_HINT: &str = "hint:summarize";
pub const THREAD_TITLE_SYSTEM_PROMPT: &str = "You generate short, specific chat thread titles from the first user message and the assistant reply. Return only the title text. Keep it under 8 words. No quotes. No markdown. No trailing punctuation unless it is part of a proper noun.";

/// Stable 16-hex-char fingerprint of a title — safe for structured logs
/// where we want to correlate events without leaking the raw title text.
pub fn title_log_fingerprint(title: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Returns `true` when the title matches the auto-generated placeholder
/// shape used by `thread_create_new` (`"Chat Mon 1 1:23 AM"` / `...PM"`).
///
/// Only placeholder titles are eligible for replacement by the LLM-generated
/// title; user-renamed threads are left untouched.
pub fn is_auto_generated_thread_title(title: &str) -> bool {
    let trimmed = title.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 16 || !trimmed.starts_with("Chat ") {
        return false;
    }

    let month_end = 8;
    if bytes.len() <= month_end || !bytes[5..month_end].iter().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    if bytes.get(month_end) != Some(&b' ') {
        return false;
    }

    let mut idx = month_end + 1;
    let day_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == day_start || idx - day_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b' ') {
        return false;
    }
    idx += 1;

    let hour_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == hour_start || idx - hour_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b':') {
        return false;
    }
    idx += 1;

    if idx + 2 >= bytes.len()
        || !bytes[idx].is_ascii_digit()
        || !bytes[idx + 1].is_ascii_digit()
        || bytes[idx + 2] != b' '
    {
        return false;
    }
    idx += 3;

    matches!(&trimmed[idx..], "AM" | "PM")
}

/// Collapses any run of whitespace (including newlines/tabs) into single
/// ASCII spaces and trims the result.
pub fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Sanitises a raw LLM title completion into a single display-ready line.
///
/// Rules applied (in order):
/// - take the first non-empty line
/// - strip wrapping quotes / backticks
/// - drop trailing `. ! ? : ;`
/// - collapse internal whitespace
/// - truncate to 80 characters
///
/// Returns `None` if the result is empty.
pub fn sanitize_generated_title(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(raw)
        .trim();
    let trimmed = line
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'))
        .trim()
        .trim_end_matches(['.', '!', '?', ':', ';'])
        .trim();
    let collapsed = collapse_whitespace(trimmed);
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(80).collect())
}

/// Builds the user-visible prompt passed to the title-generation model.
pub fn build_title_prompt(user_message: &str, assistant_message: &str) -> String {
    format!(
        "First user message:\n{user_message}\n\nAssistant reply:\n{assistant_message}\n\nReturn the best thread title."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── title_log_fingerprint ─────────────────────────────────────

    #[test]
    fn fingerprint_is_stable_for_same_input() {
        assert_eq!(
            title_log_fingerprint("hello"),
            title_log_fingerprint("hello")
        );
    }

    #[test]
    fn fingerprint_differs_for_different_input() {
        assert_ne!(
            title_log_fingerprint("hello"),
            title_log_fingerprint("world")
        );
    }

    #[test]
    fn fingerprint_is_sixteen_hex_chars() {
        let fp = title_log_fingerprint("anything");
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── is_auto_generated_thread_title ────────────────────────────

    #[test]
    fn accepts_canonical_placeholder() {
        assert!(is_auto_generated_thread_title("Chat Jan 1 1:23 AM"));
        assert!(is_auto_generated_thread_title("Chat Dec 31 11:59 PM"));
    }

    #[test]
    fn accepts_single_digit_day_and_hour() {
        assert!(is_auto_generated_thread_title("Chat Mar 5 9:07 AM"));
    }

    #[test]
    fn accepts_two_digit_day_and_hour() {
        assert!(is_auto_generated_thread_title("Chat Feb 28 10:45 PM"));
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert!(is_auto_generated_thread_title("  Chat Jan 1 1:23 AM  "));
    }

    #[test]
    fn rejects_empty_and_short_titles() {
        assert!(!is_auto_generated_thread_title(""));
        assert!(!is_auto_generated_thread_title("Chat"));
        assert!(!is_auto_generated_thread_title("Chat Jan 1"));
    }

    #[test]
    fn rejects_non_chat_prefix() {
        assert!(!is_auto_generated_thread_title("Thread Jan 1 1:23 AM"));
        assert!(!is_auto_generated_thread_title("chat Jan 1 1:23 AM")); // case matters
    }

    #[test]
    fn rejects_numeric_month() {
        assert!(!is_auto_generated_thread_title("Chat 01 1 1:23 AM"));
    }

    #[test]
    fn rejects_missing_am_pm() {
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:23"));
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:23 XM"));
    }

    #[test]
    fn rejects_user_renamed_titles() {
        assert!(!is_auto_generated_thread_title("Planning the launch party"));
        assert!(!is_auto_generated_thread_title(
            "Chat with Alice about deploys"
        ));
    }

    #[test]
    fn rejects_malformed_minutes() {
        // Minutes must be exactly two digits followed by a space.
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:2 AM"));
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:234 AM"));
    }

    // ── collapse_whitespace ────────────────────────────────────────

    #[test]
    fn collapse_whitespace_normalises_runs() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
    }

    #[test]
    fn collapse_whitespace_handles_tabs_and_newlines() {
        assert_eq!(collapse_whitespace("a\tb\nc  d"), "a b c d");
    }

    #[test]
    fn collapse_whitespace_empty_returns_empty() {
        assert_eq!(collapse_whitespace(""), "");
        assert_eq!(collapse_whitespace("   "), "");
    }

    // ── sanitize_generated_title ──────────────────────────────────

    #[test]
    fn sanitize_strips_wrapping_quotes() {
        assert_eq!(
            sanitize_generated_title("\"Launch plan\"").unwrap(),
            "Launch plan"
        );
        assert_eq!(
            sanitize_generated_title("'Debugging deploys'").unwrap(),
            "Debugging deploys"
        );
        assert_eq!(
            sanitize_generated_title("`retro notes`").unwrap(),
            "retro notes"
        );
    }

    #[test]
    fn sanitize_strips_trailing_punctuation() {
        assert_eq!(
            sanitize_generated_title("Planning session.").unwrap(),
            "Planning session"
        );
        assert_eq!(
            sanitize_generated_title("Where are we?").unwrap(),
            "Where are we"
        );
    }

    #[test]
    fn sanitize_picks_first_nonempty_line() {
        let raw = "\n\n  First real line  \nsecond line\n";
        assert_eq!(sanitize_generated_title(raw).unwrap(), "First real line");
    }

    #[test]
    fn sanitize_collapses_internal_whitespace() {
        assert_eq!(
            sanitize_generated_title("hello    world").unwrap(),
            "hello world"
        );
    }

    #[test]
    fn sanitize_returns_none_for_empty_or_whitespace() {
        assert!(sanitize_generated_title("").is_none());
        assert!(sanitize_generated_title("   \n\t  ").is_none());
        assert!(sanitize_generated_title("\"\"").is_none());
    }

    #[test]
    fn sanitize_truncates_to_eighty_chars() {
        let long = "a".repeat(200);
        let out = sanitize_generated_title(&long).unwrap();
        assert_eq!(out.chars().count(), 80);
    }

    #[test]
    fn sanitize_truncates_by_char_count_not_byte_count() {
        // Each ✨ is 3 bytes in UTF-8; ensure truncation counts chars, not bytes.
        let long: String = std::iter::repeat('✨').take(90).collect();
        let out = sanitize_generated_title(&long).unwrap();
        assert_eq!(out.chars().count(), 80);
    }

    // ── build_title_prompt ────────────────────────────────────────

    #[test]
    fn prompt_contains_both_messages_and_instruction() {
        let prompt = build_title_prompt("hello", "hi there");
        assert!(prompt.contains("First user message:\nhello"));
        assert!(prompt.contains("Assistant reply:\nhi there"));
        assert!(prompt.contains("Return the best thread title"));
    }
}
