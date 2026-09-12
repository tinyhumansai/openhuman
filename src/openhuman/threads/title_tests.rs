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
        title_from_user_message("Can you retrieve my latest 5 emails and summarize them?").unwrap(),
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
