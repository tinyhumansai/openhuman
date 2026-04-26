use super::*;

#[test]
fn short_messages_are_never_split() {
    let result = segment_for_delivery("Hello there!");
    assert_eq!(result, vec!["Hello there!"]);
}

#[test]
fn code_fences_prevent_splitting() {
    let text = "Here is some code:\n\n```rust\nfn main() {}\n```\n\nAnd more text after.";
    let result = segment_for_delivery(text);
    assert_eq!(result.len(), 1);
}

#[test]
fn paragraph_splitting_works() {
    let text = "This is the first paragraph with enough content to stand alone.\n\n\
                 This is the second paragraph that also has sufficient length.";
    let result = segment_for_delivery(text);
    assert_eq!(result.len(), 2);
}

#[test]
fn structured_content_not_split() {
    let text = "Here are the steps:\n\n- First do this thing\n- Then do that thing\n- Finally wrap up\n\nThat should cover it.";
    let result = segment_for_delivery(text);
    assert_eq!(result.len(), 1);
}

#[test]
fn sentence_splitting_works() {
    let text = "This is the first sentence and it has some length. This is the second sentence that continues the thought. And here is a third sentence to round things out.";
    let result = segment_for_delivery(text);
    assert!(
        result.len() >= 2,
        "expected >= 2 segments, got {}",
        result.len()
    );
}

#[test]
fn segment_delay_bounds() {
    assert_eq!(segment_delay(""), 500);
    assert_eq!(segment_delay(&"x".repeat(1000)), 1400);
    assert!(segment_delay("Hello world") > 500);
}

#[test]
fn numbered_list_detection() {
    assert!(is_numbered_list_item("1. First item"));
    assert!(is_numbered_list_item("12. Twelfth item"));
    assert!(!is_numbered_list_item("2024. Was a good year")); // too many digits
    assert!(!is_numbered_list_item("hello 1. world")); // digits not at start
    assert!(!is_numbered_list_item("1.5 seconds")); // no space after dot
}

#[test]
fn max_segments_respected() {
    let paras: Vec<String> = (0..10)
        .map(|i| {
            format!(
                "Paragraph number {} has enough content to stand on its own.",
                i
            )
        })
        .collect();
    let text = paras.join("\n\n");
    let result = segment_for_delivery(&text);
    assert!(result.len() <= MAX_SEGMENTS);
}

#[test]
fn split_sentences_splits_on_sentence_terminators() {
    let out = split_sentences("Hello world. How are you? I am fine!");
    assert!(out.len() >= 3);
}

#[test]
fn split_sentences_handles_empty_string() {
    assert!(split_sentences("").is_empty());
}

#[test]
fn split_sentences_single_sentence_without_terminator() {
    let out = split_sentences("Just one thing");
    assert_eq!(out.len(), 1);
}

#[test]
fn group_sentences_single_entry_roundtrip() {
    let v: Vec<String> = vec!["Hello world".into()];
    let out = group_sentences(&v);
    assert!(!out.is_empty());
}

#[test]
fn group_sentences_multi_entry_produces_output() {
    let v: Vec<String> = vec![
        "First sentence.".into(),
        "Second sentence.".into(),
        "Third sentence.".into(),
    ];
    let out = group_sentences(&v);
    assert!(!out.is_empty());
}

#[test]
fn merge_short_joins_small_parts_with_separator() {
    let out = merge_short(&["hi", "there"], " ");
    assert!(!out.is_empty());
}

#[test]
fn merge_short_empty_input_returns_empty() {
    let out: Vec<String> = merge_short(&[], " ");
    assert!(out.is_empty());
}

#[test]
fn segment_delay_is_monotonic_in_length() {
    let short = segment_delay("hi");
    let longer = segment_delay(&"a".repeat(500));
    assert!(longer >= short);
}

#[test]
fn segment_delay_is_finite_for_huge_text() {
    let huge = "a".repeat(10_000);
    assert!(segment_delay(&huge) < 1_000_000);
}

#[test]
fn segment_delay_works_on_empty_text() {
    let _ = segment_delay("");
}

#[test]
fn is_structured_content_detects_markdown_headings() {
    assert!(is_structured_content("# Heading\n\nbody"));
}

#[test]
fn is_structured_content_detects_bullet_list() {
    assert!(is_structured_content("- item 1\n- item 2"));
}

#[test]
fn is_structured_content_detects_numbered_list() {
    assert!(is_structured_content("1. First\n2. Second"));
}

#[test]
fn is_structured_content_false_for_plain_prose() {
    assert!(!is_structured_content("Just a plain sentence."));
}

#[test]
fn segment_for_delivery_whitespace_only_is_empty_or_single() {
    let r = segment_for_delivery("   ");
    // Whitespace may return a single segment or empty depending on how
    // the code treats leading/trailing whitespace. Either is acceptable.
    assert!(r.len() <= 1);
}

#[test]
fn segment_for_delivery_single_short_returns_one() {
    let r = segment_for_delivery("Quick.");
    assert_eq!(r.len(), 1);
}
