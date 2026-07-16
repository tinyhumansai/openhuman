//! Unit tests for the pure element matcher. No AX/FFI — runs on every platform.

use super::{best_match, classify, filter_and_rank, normalize, MatchTier};
use crate::openhuman::accessibility::ax_interact::AXElement;

fn el(role: &str, label: &str) -> AXElement {
    AXElement::new(role, label)
}

// --- normalize -----------------------------------------------------------

#[test]
fn normalize_lowercases_and_trims() {
    assert_eq!(normalize("  Save As  "), "save as");
}

#[test]
fn normalize_strips_trailing_ellipsis_unicode_and_ascii() {
    assert_eq!(normalize("Save As…"), "save as");
    assert_eq!(normalize("Save As..."), "save as");
    assert_eq!(normalize("Save As … "), "save as");
    // Repeated / stacked ellipsis runs collapse fully.
    assert_eq!(normalize("Export……"), "export");
}

#[test]
fn normalize_collapses_internal_whitespace() {
    assert_eq!(normalize("New\t  Folder"), "new folder");
}

#[test]
fn normalize_of_blank_is_empty() {
    assert_eq!(normalize("   "), "");
    assert_eq!(normalize("…"), "");
}

// --- classify: tiers -----------------------------------------------------

#[test]
fn classify_exact_beats_everything() {
    assert_eq!(classify("Save", "Save"), Some(MatchTier::Exact));
}

#[test]
fn classify_case_insensitive_exact() {
    assert_eq!(
        classify("Save", "save"),
        Some(MatchTier::CaseInsensitiveExact)
    );
    assert_eq!(
        classify("SAVE", "save"),
        Some(MatchTier::CaseInsensitiveExact)
    );
}

#[test]
fn classify_normalized_exact_handles_ellipsis_and_spacing() {
    assert_eq!(
        classify("Save As…", "save as"),
        Some(MatchTier::NormalizedExact)
    );
    assert_eq!(
        classify("  Save  ", "save"),
        Some(MatchTier::NormalizedExact)
    );
}

#[test]
fn classify_prefix() {
    assert_eq!(classify("Save As…", "save"), Some(MatchTier::Prefix));
}

#[test]
fn classify_word_boundary_not_prefix() {
    // "changes" starts a word inside the label but is not a prefix of it.
    assert_eq!(
        classify("Discard changes", "changes"),
        Some(MatchTier::WordBoundary)
    );
}

#[test]
fn classify_substring_is_weakest() {
    // "ave" sits mid-word — only a raw substring, not a boundary match.
    assert_eq!(classify("Save", "ave"), Some(MatchTier::Substring));
}

#[test]
fn classify_partial_word_is_substring_not_word_boundary() {
    // "changes" starts a word in "changeset" but does not end at a boundary, so
    // it must NOT rank as a word-boundary match — only the weaker substring.
    assert_eq!(
        classify("Discard changeset", "changes"),
        Some(MatchTier::Substring)
    );
}

#[test]
fn classify_non_match_is_none() {
    assert_eq!(classify("Cancel", "submit"), None);
}

#[test]
fn classify_blank_query_matches_nothing() {
    assert_eq!(classify("Anything", ""), None);
    assert_eq!(classify("Anything", "   "), None);
    // A query that is *only* an ellipsis normalizes to empty → no match.
    assert_eq!(classify("Anything", "…"), None);
}

// --- filter_and_rank -----------------------------------------------------

#[test]
fn filter_and_rank_orders_best_first() {
    let elements = vec![
        el("AXMenuItem", "Autosave"), // substring
        el("AXMenuItem", "Save As…"), // prefix
        el("AXButton", "Save"),       // case-insensitive exact
    ];
    let ranked = filter_and_rank(elements, "save");
    let labels: Vec<&str> = ranked.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, vec!["Save", "Save As…", "Autosave"]);
}

#[test]
fn filter_and_rank_drops_non_matches() {
    let elements = vec![
        el("AXButton", "Save"),
        el("AXButton", "Cancel"),
        el("AXButton", "Delete"),
    ];
    let ranked = filter_and_rank(elements, "save");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].label, "Save");
}

#[test]
fn filter_and_rank_is_stable_within_a_tier() {
    // Two equally-good substring matches keep their original tree order.
    let elements = vec![
        el("AXButton", "first tab option"),
        el("AXButton", "second tab option"),
    ];
    let ranked = filter_and_rank(elements, "option");
    let labels: Vec<&str> = ranked.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, vec!["first tab option", "second tab option"]);
}

#[test]
fn filter_and_rank_empty_query_keeps_nothing() {
    let elements = vec![el("AXButton", "Save")];
    assert!(filter_and_rank(elements, "").is_empty());
}

// --- best_match ----------------------------------------------------------

#[test]
fn best_match_none_when_no_candidates() {
    let elements = vec![el("AXButton", "Cancel")];
    assert!(best_match(&elements, "submit").is_none());
}

#[test]
fn best_match_picks_top_tier_and_is_unambiguous_when_alone() {
    let elements = vec![el("AXButton", "Save"), el("AXMenuItem", "Save As…")];
    let m = best_match(&elements, "save").expect("should match");
    assert_eq!(m.element.label, "Save");
    assert_eq!(m.tier, MatchTier::CaseInsensitiveExact);
    // The other candidate is a weaker (Prefix) tier, so the top tier is a clean win.
    assert!(!m.ambiguous);
}

#[test]
fn best_match_flags_ambiguity_on_a_same_tier_tie() {
    // Both are prefix matches for "save" but name different controls.
    let elements = vec![el("AXMenuItem", "Save As…"), el("AXMenuItem", "Save All")];
    let m = best_match(&elements, "save").expect("should match");
    assert_eq!(m.tier, MatchTier::Prefix);
    assert!(m.ambiguous);
}

#[test]
fn best_match_same_tier_same_label_is_not_ambiguous() {
    // Both land in the SAME non-trivial tier (NormalizedExact — neither is a
    // literal Exact/CaseInsensitiveExact hit) yet normalize identically, so the
    // same-tier rival comparison in `best_match` runs and finds no real rival.
    let elements = vec![el("AXButton", "Save As…"), el("AXMenuItem", "Save   As")];
    let m = best_match(&elements, "Save As").expect("should match");
    assert_eq!(m.tier, MatchTier::NormalizedExact);
    assert!(!m.ambiguous);
}

#[test]
fn best_match_prefers_exact_over_ambiguous_weaker_matches() {
    let elements = vec![
        el("AXMenuItem", "Save As…"),
        el("AXButton", "Save"),
        el("AXMenuItem", "Save All"),
    ];
    let m = best_match(&elements, "Save").expect("should match");
    assert_eq!(m.element.label, "Save");
    assert_eq!(m.tier, MatchTier::Exact);
    // Exact tier has exactly one member even though weaker tiers are contested.
    assert!(!m.ambiguous);
}
