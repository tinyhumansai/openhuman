//! Tests for `learning::cache::FacetCache`.

use parking_lot::Mutex;
use rusqlite::Connection;
use std::sync::Arc;

use super::*;
use crate::openhuman::learning::candidate::{EvidenceRef, FacetClass};
use crate::openhuman::memory_store::profile::{
    FacetState, FacetType, ProfileFacet, UserState, PROFILE_INIT_SQL,
};

fn make_cache() -> FacetCache {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(PROFILE_INIT_SQL).unwrap();
    FacetCache::new(Arc::new(Mutex::new(conn)))
}

fn stub_facet(id: &str, key: &str, value: &str, state: FacetState, stability: f64) -> ProfileFacet {
    ProfileFacet {
        facet_id: id.into(),
        facet_type: FacetType::Preference,
        key: key.into(),
        value: value.into(),
        confidence: 0.8,
        evidence_count: 2,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1200.0,
        state,
        stability,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: None,
        cue_families: None,
    }
}

// ── upsert_then_list_active ───────────────────────────────────────────────────

#[test]
fn upsert_then_list_active() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f1",
            "style/verbosity",
            "terse",
            FacetState::Active,
            1.8,
        ))
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f2",
            "style/tone",
            "formal",
            FacetState::Provisional,
            0.8,
        ))
        .unwrap();

    let active = cache.list_active().unwrap();
    assert_eq!(active.len(), 1, "only Active state should be listed");
    assert_eq!(active[0].key, "style/verbosity");
}

// ── class_from_key_parses_known_classes ───────────────────────────────────────

#[test]
fn class_from_key_parses_known_classes() {
    assert_eq!(class_from_key("style/verbosity"), Some(FacetClass::Style));
    assert_eq!(class_from_key("identity/name"), Some(FacetClass::Identity));
    assert_eq!(
        class_from_key("tooling/package_manager"),
        Some(FacetClass::Tooling)
    );
    assert_eq!(
        class_from_key("veto/no_sports_updates"),
        Some(FacetClass::Veto)
    );
    assert_eq!(class_from_key("goal/learn_rust"), Some(FacetClass::Goal));
    assert_eq!(class_from_key("channel/slack"), Some(FacetClass::Channel));
    assert_eq!(class_from_key("unknown/foo"), None);
    assert_eq!(class_from_key("no_slash"), None);
}

// ── set_user_state_pinned_persists ────────────────────────────────────────────

#[test]
fn set_user_state_pinned_persists() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f-pin",
            "identity/name",
            "Alice",
            FacetState::Active,
            2.0,
        ))
        .unwrap();

    let updated = cache
        .set_user_state("identity/name", UserState::Pinned)
        .unwrap();
    assert!(updated, "row should exist and be updated");

    let f = cache.get("identity/name").unwrap().unwrap();
    assert_eq!(f.user_state, UserState::Pinned);
}

// ── drop_below_threshold_removes_facets ───────────────────────────────────────

#[test]
fn drop_below_threshold_removes_facets() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f-low",
            "style/dropped_one",
            "x",
            FacetState::Dropped,
            0.1,
        ))
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f-keep",
            "style/active_one",
            "y",
            FacetState::Active,
            0.1, // low stability but Active state — should NOT be deleted
        ))
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f-pinned-drop",
            "style/pinned_one",
            "z",
            FacetState::Dropped,
            0.1,
        ))
        .and_then(|_| cache.set_user_state("style/pinned_one", UserState::Pinned))
        .unwrap();

    let removed = cache.drop_below_threshold(0.3).unwrap();
    assert_eq!(
        removed, 1,
        "only the non-pinned Dropped row should be removed"
    );

    // Active and Pinned rows survive.
    let all = cache.list_all().unwrap();
    assert_eq!(all.len(), 2);
}

// ── list_by_class_filters_correctly ───────────────────────────────────────────

#[test]
fn list_by_class_filters_correctly() {
    let cache = make_cache();

    for (id, key, val) in [
        ("f-s1", "style/verbosity", "terse"),
        ("f-s2", "style/tone", "formal"),
        ("f-i1", "identity/name", "Alice"),
    ] {
        cache
            .upsert(&stub_facet(id, key, val, FacetState::Active, 1.6))
            .unwrap();
    }

    let style = cache.list_by_class(FacetClass::Style).unwrap();
    assert_eq!(style.len(), 2);
    assert!(style.iter().all(|f| f.key.starts_with("style/")));

    let identity = cache.list_by_class(FacetClass::Identity).unwrap();
    assert_eq!(identity.len(), 1);
    assert_eq!(identity[0].key, "identity/name");

    let tooling = cache.list_by_class(FacetClass::Tooling).unwrap();
    assert!(tooling.is_empty());
}

// ── key_with_class helper ─────────────────────────────────────────────────────

#[test]
fn key_with_class_produces_prefixed_key() {
    assert_eq!(
        key_with_class(FacetClass::Style, "verbosity"),
        "style/verbosity"
    );
    assert_eq!(
        key_with_class(FacetClass::Tooling, "package_manager"),
        "tooling/package_manager"
    );
}

// ── Evidence refs round-trip ──────────────────────────────────────────────────

#[test]
fn evidence_refs_survive_upsert_round_trip() {
    let cache = make_cache();
    let mut f = stub_facet("f-ev", "identity/email", "a@b.com", FacetState::Active, 2.0);
    f.evidence_refs = vec![
        EvidenceRef::Provider {
            toolkit: "gmail".into(),
            connection_id: "c-1".into(),
            field: "email".into(),
        },
        EvidenceRef::Episodic { episodic_id: 7 },
    ];
    cache.upsert(&f).unwrap();

    let loaded = cache.get("identity/email").unwrap().unwrap();
    assert_eq!(loaded.evidence_refs.len(), 2);
    assert_eq!(
        loaded.evidence_refs[0],
        EvidenceRef::Provider {
            toolkit: "gmail".into(),
            connection_id: "c-1".into(),
            field: "email".into(),
        }
    );
}

// ── delete helper ─────────────────────────────────────────────────────────────

#[test]
fn delete_removes_facet_by_key() {
    let cache = make_cache();
    cache
        .upsert(&stub_facet(
            "f-del",
            "goal/learn_rust",
            "learn Rust",
            FacetState::Active,
            1.5,
        ))
        .unwrap();

    let deleted = cache.delete("goal/learn_rust").unwrap();
    assert!(deleted);

    let loaded = cache.get("goal/learn_rust").unwrap();
    assert!(loaded.is_none());
}
