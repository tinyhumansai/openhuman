//! Step 6 — the pure char-budget arithmetic.

use super::*;
use crate::openhuman::memory::api::types::{MemoryCategory, MemoryEntry, MemoryTaint};

fn entry(content: &str) -> MemoryEntry {
    MemoryEntry {
        id: "id".into(),
        key: "key".into(),
        content: content.into(),
        namespace: Some("ns".into()),
        category: MemoryCategory::Core,
        timestamp: "2026-01-01T00:00:00Z".into(),
        session_id: None,
        score: None,
        taint: MemoryTaint::Internal,
    }
}

#[test]
fn truncate_content_leaves_a_fitting_string_untouched() {
    let out = truncate_content("hello", 5);
    assert_eq!(out, "hello");
    assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn truncate_content_cuts_to_the_budget() {
    assert_eq!(truncate_content("hello world", 5), "hello");
}

#[test]
fn guard_budget_counts_chars_not_bytes() {
    // Six chars, eighteen bytes. A byte-counting implementation would cut this
    // to two chars — or panic on a non-char-boundary index.
    let multibyte = "日本語です、はい";
    assert_eq!(truncate_content(multibyte, 3), "日本語");
    assert_eq!(truncate_content(multibyte, 100), multibyte);
}

#[test]
fn guard_truncates_recall_results_to_the_budget() {
    let out = truncate_entries(vec![entry("aaaa"), entry("bbbb"), entry("cccc")], 6);
    assert_eq!(
        out.entries.len(),
        2,
        "the straddling entry is kept, trimmed"
    );
    assert_eq!(out.entries[0].content, "aaaa");
    assert_eq!(out.entries[1].content, "bb");
    assert_eq!(out.dropped, 1);
    assert_eq!(out.trimmed_chars, 2 + 4);
}

#[test]
fn a_budget_that_fits_changes_nothing() {
    let out = truncate_entries(vec![entry("aaaa"), entry("bbbb")], 100);
    assert_eq!(out.entries.len(), 2);
    assert_eq!(out.dropped, 0);
    assert_eq!(out.trimmed_chars, 0);
}

#[test]
fn a_zero_budget_drops_everything_when_the_caller_asks_for_one() {
    // `GuardPolicy::recall_budget` never passes 0 (it reads 0 as "disabled"),
    // but the pure function must still be total rather than panicking.
    let out = truncate_entries(vec![entry("aaaa")], 0);
    assert!(out.entries.is_empty());
    assert_eq!(out.dropped, 1);
}

#[test]
fn budget_spends_in_rank_order_so_the_top_hit_survives_whole() {
    let out = truncate_entries(vec![entry("top hit"), entry("second")], 7);
    assert_eq!(out.entries[0].content, "top hit");
    assert_eq!(out.dropped, 1);
}
