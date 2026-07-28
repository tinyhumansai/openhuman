//! Tests for recall content condensation.

use super::*;

/// One store chunk is ~225 tokens ≈ 900 chars. Size a paragraph just under
/// that so each `para` is exactly one chunk and "section" counts are intuitive.
fn para(tag: &str) -> String {
    format!("{tag} {}", "filler ".repeat(90))
}

#[test]
fn short_content_passes_through_untouched() {
    let fact = "User is the PI on the Colorado collaboration.";
    assert_eq!(condense_recall_content("colorado", fact), fact);
}

#[test]
fn only_the_query_relevant_chunks_survive() {
    // Five distinct large sections; only two mention the needle.
    let content = [
        para("alpha unrelated"),
        para("beta 콜로라도 대학 연구"),
        para("gamma unrelated"),
        para("delta unrelated"),
        para("epsilon 콜로라도 공동연구"),
    ]
    .join("\n\n");

    let out = condense_recall_content("콜로라도 연구", &content);

    assert!(
        out.contains("beta"),
        "a matching section must be kept: {out:.120}"
    );
    assert!(out.contains("epsilon"), "the other match must be kept too");
    // An unrelated section is dropped, and the drop is disclosed.
    assert!(
        !out.contains("gamma"),
        "an unrelated section must be dropped"
    );
    assert!(
        out.contains("more section(s)"),
        "the elision is disclosed: {out:.80}"
    );
}

#[test]
fn at_most_three_chunks_from_one_source() {
    // Six sections all match — the cap, not relevance, must bound the output.
    let content = (0..6)
        .map(|i| para(&format!("sec{i} 콜로라도")))
        .collect::<Vec<_>>()
        .join("\n\n");

    let out = condense_recall_content("콜로라도", &content);
    let kept = (0..6).filter(|i| out.contains(&format!("sec{i}"))).count();
    assert!(
        kept <= MAX_CHUNKS_PER_SOURCE,
        "kept {kept} sections, cap is {MAX_CHUNKS_PER_SOURCE}"
    );
    assert!(kept > 0, "at least the matching sections are shown");
    assert!(
        out.contains("more section(s)"),
        "the dropped sections are disclosed: {out:.80}"
    );
}

#[test]
fn an_unsplittable_blob_is_hard_capped() {
    // No paragraph breaks: chunk selection can't help, so the char cap is the
    // only thing standing between a 20 KB blob and the result budget.
    let blob = "x".repeat(20_000);
    let out = condense_recall_content("anything", &blob);
    assert!(
        out.chars().count() <= MAX_CHARS_PER_ENTRY,
        "a blob must be truncated to the char cap, got {} chars",
        out.chars().count()
    );
}

#[test]
fn the_system_prompt_envelope_no_longer_floods_the_result() {
    // The real regression: a subagent's system-prompt envelope saved as a
    // conversation. Whatever its size, one entry must not exceed the cap.
    let envelope = format!(
        "{{\"_meta\":{{\"agent\":\"integrations_agent\"}}}}\n{}",
        para("You are the Integrations Agent").repeat(30)
    );
    let out = condense_recall_content("콜로라도 대학 연구", &envelope);
    assert!(
        out.chars().count() <= MAX_CHARS_PER_ENTRY,
        "envelope exceeded the cap"
    );
}
