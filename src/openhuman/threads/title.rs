//! Pure helpers for generating and validating conversation thread titles.
//!
//! Extracted from `threads::ops` so the parsing / sanitisation rules can be
//! unit-tested without pulling in `Config`, provider runtime, or RPC wiring.

use std::hash::{Hash, Hasher};

pub const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
pub const THREAD_TITLE_MODEL_HINT: &str = "hint:summarize";
pub const THREAD_TITLE_SYSTEM_PROMPT: &str = "You name chat threads from the first user message and the assistant reply. Return only the name: at most 3 words, like Fix session handoff or Gmail OAuth retry. Lead with the verb or the subject and drop filler words. No quotes. No markdown. No punctuation.";

/// Words a title carries at most. Three is the whole point of the shape: a
/// thread list is scanned, not read, and a fourth word is always the one that
/// pushes the specific words off the end of a narrow row.
pub const THREAD_TITLE_MAX_WORDS: usize = 3;
/// Hard character ceiling on a title, so one very long word cannot widen a row.
pub const THREAD_TITLE_MAX_CHARS: usize = 48;

/// Filler a title is better off without.
///
/// Prompts open with conversational scaffolding ("okay so can you please…"),
/// and taking the first three words verbatim would spend the whole title on it.
/// Only words that never identify a thread on their own are listed; a filtered
/// title that comes out empty falls back to the unfiltered words, so a message
/// made entirely of these still gets a name.
const FILLER_WORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "do", "does",
    "for", "from", "hey", "hi", "how", "i", "if", "in", "into", "is", "it", "its", "just", "let",
    "lets", "like", "me", "my", "of", "ok", "okay", "on", "or", "our", "please", "so", "thanks",
    "that", "the", "their", "then", "there", "these", "they", "this", "to", "uh", "um", "us",
    "was", "we", "well", "what", "when", "which", "will", "with", "would", "you", "your",
];

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

/// Reduces any text to the thread-title shape: at most
/// [`THREAD_TITLE_MAX_WORDS`] words, e.g. `Fix session handoff`.
///
/// This is the shape enforcer, not a request: the model is asked for a short
/// name in [`THREAD_TITLE_SYSTEM_PROMPT`], but a title that reaches storage as
/// a whole sentence because one completion ignored the instruction is exactly
/// the bug the shape is meant to remove, so every path runs through here.
///
/// Rules applied (in order):
/// - split on anything that is not alphanumeric (punctuation, quotes, markdown,
///   and whitespace all become word breaks)
/// - drop [`FILLER_WORDS`], unless that would leave nothing
/// - keep the first [`THREAD_TITLE_MAX_WORDS`] words, and stop early rather
///   than exceed [`THREAD_TITLE_MAX_CHARS`]
///
/// A word's own spelling is left alone — `OAuth` and `Gmail` read wrong
/// lowercased, and the model is the only thing here that knows which is which.
///
/// Returns `None` when no word survives.
pub fn shorten_title(text: &str) -> Option<String> {
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let meaningful: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| !FILLER_WORDS.contains(&word.to_lowercase().as_str()))
        .collect();
    // All-filler input ("can you please") still names its thread, badly-but-
    // stably, rather than leaving the placeholder title in place.
    let source = if meaningful.is_empty() {
        words
    } else {
        meaningful
    };

    let mut title = String::new();
    for word in source.into_iter().take(THREAD_TITLE_MAX_WORDS) {
        let separator = usize::from(!title.is_empty());
        let room = THREAD_TITLE_MAX_CHARS.saturating_sub(title.chars().count() + separator);
        if room == 0 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        // A single word longer than the ceiling is truncated rather than
        // dropped: dropping it can empty an otherwise usable title.
        title.extend(word.chars().take(room));
    }
    (!title.is_empty()).then_some(title)
}

/// Sanitises a raw LLM title completion into a stored thread title.
///
/// Takes the first non-empty line — a chatty model that adds a second line of
/// commentary should not have it folded into the name — and shortens it with
/// [`shorten_title`], which absorbs the quote/markdown/punctuation stripping
/// the older sentence-shaped title needed done by hand.
///
/// Returns `None` if the result is empty.
pub fn sanitize_generated_title(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(raw);
    shorten_title(line)
}

/// Derives a stable display title directly from the first useful user message.
///
/// This is the no-provider fallback used while a conversation only has user
/// context, or when model-based title generation is unavailable. It keeps the
/// title meaningful without repeatedly renaming the thread later.
pub fn title_from_user_message(message: &str) -> Option<String> {
    let collapsed = collapse_whitespace(message);
    if collapsed.is_empty() {
        return None;
    }

    // Only the first sentence describes the ask; what follows is context the
    // title has no room for anyway.
    let first_sentence = collapsed
        .split(['.', '!', '?', '\n'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(&collapsed);
    shorten_title(first_sentence)
}

/// Builds the user-visible prompt passed to the title-generation model.
pub fn build_title_prompt(user_message: &str, assistant_message: &str) -> String {
    format!(
        "First user message:\n{user_message}\n\nAssistant reply:\n{assistant_message}\n\nReturn the best thread name."
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
        // Lowercase hex specifically, so grep-friendly debug logs stay
        // consistent (folded in from the former threads/ops_tests copy).
        assert!(
            fp.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "fingerprint must be lowercase hex, got: {fp}"
        );
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

    // ── shorten_title ─────────────────────────────────────────────

    #[test]
    fn shorten_keeps_at_most_three_words() {
        assert_eq!(
            shorten_title("Fix session handoff flow and pointer").unwrap(),
            "Fix session handoff"
        );
        assert_eq!(shorten_title("Launch Plan").unwrap(), "Launch Plan");
    }

    #[test]
    fn shorten_leaves_a_words_own_spelling_alone() {
        // Lowercasing would turn these into something nobody would type.
        assert_eq!(
            shorten_title("Gmail OAuth retry loop").unwrap(),
            "Gmail OAuth retry"
        );
    }

    #[test]
    fn shorten_drops_leading_filler() {
        assert_eq!(
            shorten_title("okay so can you please fix the session handoff").unwrap(),
            "fix session handoff"
        );
    }

    #[test]
    fn shorten_falls_back_to_filler_when_that_is_all_there_is() {
        assert_eq!(shorten_title("can you please").unwrap(), "can you please");
    }

    #[test]
    fn shorten_treats_punctuation_and_markdown_as_word_breaks() {
        assert_eq!(
            shorten_title("**Debugging deploys:** retry").unwrap(),
            "Debugging deploys retry"
        );
        assert_eq!(
            shorten_title("\"gmail/oauth retry\"").unwrap(),
            "gmail oauth retry"
        );
    }

    #[test]
    fn shorten_bounds_total_length() {
        let long = format!("{} {} {}", "a".repeat(30), "b".repeat(30), "c".repeat(30));
        let out = shorten_title(&long).unwrap();
        assert!(out.chars().count() <= THREAD_TITLE_MAX_CHARS);
        // A word that does not fit whole is truncated, never dropped.
        assert!(out.starts_with(&"a".repeat(30)));
    }

    #[test]
    fn shorten_counts_chars_not_bytes() {
        // Each ✨ is 3 bytes in UTF-8, and is not alphanumeric — a title made
        // only of them has no word to keep.
        assert!(shorten_title(&"✨".repeat(90)).is_none());
        let out = shorten_title(&"é".repeat(90)).unwrap();
        assert_eq!(out.chars().count(), THREAD_TITLE_MAX_CHARS);
    }

    #[test]
    fn shorten_returns_none_without_a_word() {
        assert!(shorten_title("").is_none());
        assert!(shorten_title("   \n\t  ").is_none());
        assert!(shorten_title("///").is_none());
    }

    // ── sanitize_generated_title ──────────────────────────────────

    #[test]
    fn sanitize_shortens_a_sentence_shaped_completion() {
        assert_eq!(
            sanitize_generated_title("\"Planning the launch party\"").unwrap(),
            "Planning launch party"
        );
        // "are"/"we" are filler, so a question collapses to its one real word.
        assert_eq!(sanitize_generated_title("Where are we?").unwrap(), "Where");
    }

    #[test]
    fn sanitize_passes_an_already_short_completion_through() {
        assert_eq!(
            sanitize_generated_title("Fix session handoff").unwrap(),
            "Fix session handoff"
        );
    }

    #[test]
    fn sanitize_picks_first_nonempty_line() {
        let raw = "\n\n  First real line  \nsecond line\n";
        assert_eq!(sanitize_generated_title(raw).unwrap(), "First real line");
    }

    #[test]
    fn sanitize_returns_none_for_empty_or_whitespace() {
        assert!(sanitize_generated_title("").is_none());
        assert!(sanitize_generated_title("   \n\t  ").is_none());
        assert!(sanitize_generated_title("\"\"").is_none());
    }

    #[test]
    fn sanitize_bounds_length() {
        let long = "a".repeat(200);
        let out = sanitize_generated_title(&long).unwrap();
        assert_eq!(out.chars().count(), THREAD_TITLE_MAX_CHARS);
    }

    // ── title_from_user_message ──────────────────────────────────

    #[test]
    fn title_from_user_message_uses_first_specific_words() {
        assert_eq!(
            title_from_user_message("Can you retrieve my latest 5 emails and summarize them?")
                .unwrap(),
            "retrieve latest 5"
        );
    }

    #[test]
    fn title_from_user_message_removes_command_prefix_and_punctuation() {
        assert_eq!(
            title_from_user_message("/briefing Morning update, please. Then check email").unwrap(),
            "briefing Morning update"
        );
    }

    #[test]
    fn title_from_user_message_returns_none_for_empty_context() {
        assert!(title_from_user_message("   \n\t  ").is_none());
        assert!(title_from_user_message("///").is_none());
    }

    // ── build_title_prompt ────────────────────────────────────────

    #[test]
    fn prompt_contains_both_messages_and_instruction() {
        let prompt = build_title_prompt("hello", "hi there");
        assert!(prompt.contains("First user message:\nhello"));
        assert!(prompt.contains("Assistant reply:\nhi there"));
        assert!(prompt.contains("Return the best thread name"));
    }
}
