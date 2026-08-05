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

/// With fewer matches than slots, the leftover slots used to be filled by
/// whatever sorted next — which, once anything matches, is unrelated text. The
/// model then read a passage the query never asked for beside one it did.
#[test]
fn unmatched_sections_do_not_pad_out_the_remaining_slots() {
    let content = [
        para("alpha unrelated"),
        para("beta 콜로라도 대학 연구"),
        para("gamma unrelated"),
        para("delta unrelated"),
        para("epsilon unrelated"),
    ]
    .join("\n\n");

    let out = condense_recall_content("콜로라도", &content);

    assert!(
        out.contains("beta"),
        "the only match must be kept: {out:.120}"
    );
    for unrelated in ["alpha", "gamma", "delta", "epsilon"] {
        assert!(
            !out.contains(unrelated),
            "`{unrelated}` does not match the query and must not fill a slot: {out:.200}"
        );
    }
}

/// The fallback the change above must not break: with nothing matching there is
/// no better answer than the head of the document, so the slots are still used.
#[test]
fn nothing_matching_still_returns_the_head_of_the_document() {
    let content = (0..5)
        .map(|i| para(&format!("sec{i} unrelated")))
        .collect::<Vec<_>>()
        .join("\n\n");

    let out = condense_recall_content("콜로라도", &content);
    let kept = (0..5).filter(|i| out.contains(&format!("sec{i}"))).count();

    assert_eq!(
        kept, MAX_CHUNKS_PER_SOURCE,
        "with no match the cap still fills from the top: {out:.200}"
    );
    assert!(out.contains("sec0"), "and starts at the head: {out:.120}");
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

/// The cap is a cap. `truncate_with_ellipsis` adds its "..." on top of the
/// budget it is handed, so a hard cap that forgets to reserve those characters
/// returns text longer than the limit it was asked to enforce — silently, and on
/// every truncated recall entry.
#[test]
fn hard_cap_counts_the_ellipsis_against_the_budget() {
    let long = "x".repeat(500);
    for max in [10usize, 33, 120] {
        let capped = super::hard_cap(&long, max);
        assert!(
            capped.chars().count() <= max,
            "hard_cap({max}) returned {} chars: {capped}",
            capped.chars().count()
        );
    }
    // Text that already fits is returned whole, ellipsis or not.
    assert_eq!(super::hard_cap("short", 32), "short");
}
