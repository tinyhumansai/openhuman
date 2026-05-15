//! Additional unit tests for `learning::prompt_sections` — specifically the
//! `load_learned_from_cache` top-K ranking cap and pinned-facet rendering,
//! not covered by the inline tests in `prompt_sections.rs`.

use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

use super::load_learned_from_cache;
use crate::openhuman::learning::cache::FacetCache;
use crate::openhuman::memory::store::profile::{
    FacetState, FacetType, ProfileFacet, UserState, PROFILE_INIT_SQL,
};

fn open_cache() -> FacetCache {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(PROFILE_INIT_SQL).unwrap();
    FacetCache::new(Arc::new(Mutex::new(conn)))
}

fn make_active(id: &str, key: &str, value: &str, stability: f64) -> ProfileFacet {
    ProfileFacet {
        facet_id: id.into(),
        facet_type: FacetType::Preference,
        key: key.into(),
        value: value.into(),
        confidence: 0.9,
        evidence_count: 3,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 2000.0,
        state: FacetState::Active,
        stability,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: None,
        cue_families: None,
    }
}

// ── Top-K cap (CACHE_PROMPT_CAP = 25) ────────────────────────────────────────

/// When more than 25 Active facets exist, output is capped at 25 entries.
#[test]
fn load_learned_from_cache_caps_at_25_entries() {
    let cache = open_cache();

    // Insert 30 active style facets.
    for i in 0..30u32 {
        cache
            .upsert(&make_active(
                &format!("f{i}"),
                &format!("style/key_{i:02}"),
                &format!("val{i}"),
                1.5 + (i as f64) * 0.01,
            ))
            .unwrap();
    }

    let result = load_learned_from_cache(&cache);
    assert_eq!(
        result.len(),
        25,
        "output must be capped at CACHE_PROMPT_CAP=25; got {}",
        result.len()
    );
}

// ── Stability ranking ─────────────────────────────────────────────────────────

/// Within the same class, higher-stability facets appear before lower ones.
#[test]
fn load_learned_from_cache_ranks_by_stability_descending() {
    let cache = open_cache();

    cache
        .upsert(&make_active("f-lo", "style/low_stab", "lo", 0.5))
        .unwrap();
    cache
        .upsert(&make_active("f-hi", "style/high_stab", "hi", 2.5))
        .unwrap();
    cache
        .upsert(&make_active("f-mid", "style/mid_stab", "mid", 1.5))
        .unwrap();

    let result = load_learned_from_cache(&cache);
    assert!(!result.is_empty());

    // Find positions of high / low in the result list.
    let pos_hi = result
        .iter()
        .position(|s| s.contains("high_stab"))
        .expect("high_stab should appear");
    let pos_lo = result
        .iter()
        .position(|s| s.contains("low_stab"))
        .expect("low_stab should appear");

    assert!(
        pos_hi < pos_lo,
        "higher-stability facet should appear before lower-stability; positions hi={pos_hi} lo={pos_lo}"
    );
}

// ── Pinned marker ─────────────────────────────────────────────────────────────

/// Pinned facets must carry the `*(pinned)*` marker in the output.
#[test]
fn load_learned_from_cache_marks_pinned_facets() {
    let cache = open_cache();

    cache
        .upsert(&make_active("f-pin", "identity/name", "Alice", 2.0))
        .unwrap();
    cache
        .set_user_state("identity/name", UserState::Pinned)
        .unwrap();

    let result = load_learned_from_cache(&cache);
    let pinned_entry = result
        .iter()
        .find(|s| s.contains("identity/name"))
        .expect("identity/name should appear");
    assert!(
        pinned_entry.contains("*(pinned)*"),
        "pinned facet must include marker; got: {pinned_entry}"
    );
}

// ── Dropped state excluded ────────────────────────────────────────────────────

/// Dropped-state facets must not appear even when their stability is high.
#[test]
fn load_learned_from_cache_excludes_dropped_facets() {
    let cache = open_cache();

    let mut dropped = make_active("f-drop", "style/dropped", "x", 3.0);
    dropped.state = FacetState::Dropped;
    cache.upsert(&dropped).unwrap();

    let result = load_learned_from_cache(&cache);
    assert!(
        !result.iter().any(|s| s.contains("style/dropped")),
        "dropped facet must not appear in output"
    );
}

// ── Multi-class ordering ──────────────────────────────────────────────────────

/// When multiple classes are present, output is grouped by class (BTreeMap
/// order — alphabetical: channel, goal, identity, style, tooling, veto).
/// We only assert that facets from every class are present.
#[test]
fn load_learned_from_cache_includes_facets_from_all_classes() {
    let cache = open_cache();

    let entries = [
        ("fc", "channel/slack", "Slack"),
        ("fg", "goal/learn_rust", "Learn Rust"),
        ("fi", "identity/name", "Alice"),
        ("fs", "style/verbosity", "terse"),
        ("ft", "tooling/pkg_mgr", "pnpm"),
        ("fv", "veto/no_sports", "true"),
    ];
    for (id, key, val) in &entries {
        cache.upsert(&make_active(id, key, val, 1.8)).unwrap();
    }

    let result = load_learned_from_cache(&cache);

    // Goal class renders value-only; others render "**key**: value".
    assert!(result.iter().any(|s| s.contains("Learn Rust")));
    assert!(result.iter().any(|s| s.contains("identity/name")));
    assert!(result.iter().any(|s| s.contains("style/verbosity")));
    assert!(result.iter().any(|s| s.contains("tooling/pkg_mgr")));
    assert!(result.iter().any(|s| s.contains("veto/no_sports")));
    assert!(result.iter().any(|s| s.contains("channel/slack")));
}

// ── Empty-cache short-circuit ─────────────────────────────────────────────────

/// An empty cache (no Active facets) must return an empty vec, not an error.
#[test]
fn load_learned_from_cache_returns_empty_for_empty_cache() {
    let cache = open_cache();
    assert!(load_learned_from_cache(&cache).is_empty());
}

// ── drop_below_threshold does not touch Active rows ───────────────────────────

/// Eviction via `FacetCache::drop_below_threshold` must leave Active rows
/// untouched regardless of their stability value.
#[test]
fn drop_below_threshold_skips_active_rows() {
    let cache = open_cache();

    // Insert an Active row with very low stability — it must survive eviction.
    cache
        .upsert(&make_active("f-active-low", "style/keep_me", "v", 0.01))
        .unwrap();

    let removed = cache.drop_below_threshold(10.0).unwrap(); // aggressive threshold
    assert_eq!(removed, 0, "Active rows must never be evicted");

    let entry = cache.get("style/keep_me").unwrap();
    assert!(
        entry.is_some(),
        "Active row must still exist after eviction"
    );
}
