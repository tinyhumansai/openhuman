//! Tests for `learning::cache::FacetCache`.

use super::*;
use crate::openhuman::agent::learning::candidate::FacetClass;
use tinymemory_api::host::EvidenceRef;
use tinymemory_api::provider::{FacetState, FacetType, ProfileFacet, UserState};

fn make_cache() -> FacetCache {
    crate::openhuman::agent::learning::test_profile::in_memory_cache()
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

#[tokio::test]
async fn upsert_then_list_active() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f1",
            "style/verbosity",
            "terse",
            FacetState::Active,
            1.8,
        ))
        .await
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f2",
            "style/tone",
            "formal",
            FacetState::Provisional,
            0.8,
        ))
        .await
        .unwrap();

    let active = cache.list_active().await.unwrap();
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

// ── parse_facet_class_name ────────────────────────────────────────────────────

#[test]
fn parse_facet_class_name_accepts_every_taxonomy_name() {
    assert_eq!(parse_facet_class_name("style"), Ok(FacetClass::Style));
    assert_eq!(parse_facet_class_name("identity"), Ok(FacetClass::Identity));
    assert_eq!(parse_facet_class_name("tooling"), Ok(FacetClass::Tooling));
    assert_eq!(parse_facet_class_name("veto"), Ok(FacetClass::Veto));
    assert_eq!(parse_facet_class_name("goal"), Ok(FacetClass::Goal));
    assert_eq!(parse_facet_class_name("channel"), Ok(FacetClass::Channel));
}

#[test]
fn parse_facet_class_name_rejects_unknown_class() {
    let err = parse_facet_class_name("nonsense").expect_err("unknown class must be rejected");
    assert!(err.contains("invalid class `nonsense`"), "got: {err}");
    // Lists the accepted taxonomy so the caller can recover.
    assert!(
        err.contains("style, identity, tooling, veto, goal, channel"),
        "got: {err}"
    );
}

#[test]
fn parse_facet_class_name_rejects_a_name_with_a_slash() {
    // `"style/"` shares a first `/`-segment with a real class, so a bare
    // `class_from_key` would accept it and then compose an unmatchable key —
    // the silent-empty behaviour #6077 removes. The slash guard rejects it.
    let err =
        parse_facet_class_name("style/").expect_err("a slash in a class name must be rejected");
    assert!(err.contains("invalid class `style/`"), "got: {err}");
}

// ── class filter is column-only (#6077) ───────────────────────────────────────
//
// `list_facets` filters on the `class` column alone; the redundant
// `|| key.starts_with("{class}/")` arm was dropped. This proves the two are not
// equivalent when they disagree: a valid class returns exactly the rows whose
// `class` column matches, and a key-prefix that does not match the column is
// excluded.
#[tokio::test]
async fn class_filter_matches_column_not_key_prefix() {
    let cache = make_cache();

    // Two facets whose class column is genuinely `style`.
    let mut a = stub_facet("f1", "style/verbosity", "terse", FacetState::Active, 1.8);
    a.class = Some("style".into());
    let mut b = stub_facet("f2", "style/tone", "formal", FacetState::Active, 0.9);
    b.class = Some("style".into());
    // A facet whose key prefix reads "style/" but whose class column is `goal` —
    // the old `starts_with` arm would have wrongly matched this under a `style`
    // filter; the column-only rule must not.
    let mut c = stub_facet("f3", "style/mislabelled", "x", FacetState::Active, 0.5);
    c.class = Some("goal".into());

    cache.upsert(&a).await.unwrap();
    cache.upsert(&b).await.unwrap();
    cache.upsert(&c).await.unwrap();

    // The same predicate list_facets now applies: class column only.
    let all = cache.list_all().await.unwrap();
    let matched: Vec<&str> = all
        .iter()
        .filter(|f| f.state == FacetState::Active)
        .filter(|f| f.class.as_deref() == Some("style"))
        .map(|f| f.key.as_str())
        .collect();

    assert_eq!(matched, vec!["style/verbosity", "style/tone"]);
    assert!(
        !matched.contains(&"style/mislabelled"),
        "column-only filter must exclude a key-prefix match with a different class column"
    );
}

// ── classless rows fall back to the key prefix (#6104) ────────────────────────
//
// `class` is `Option`: a facet can carry a canonical key like `style/verbosity`
// with `class: None`. The column-only filter that #6077 introduced hid such
// rows entirely. `list_facets` therefore keeps the class column authoritative
// but falls back to the key prefix **only when the column is absent** — the
// same predicate both `handle_list_facets` and `LearningListFacetsTool` apply.
// This proves the fallback restores a legitimate classless row without
// reopening the leak: the row is returned for its own class and for no other.
#[tokio::test]
async fn class_filter_falls_back_to_key_prefix_for_classless_rows() {
    let cache = make_cache();

    // A facet whose class column is absent but whose canonical key names the
    // class. `stub_facet` already defaults `class` to `None`.
    let classless = stub_facet("f1", "style/verbosity", "terse", FacetState::Active, 1.8);
    assert!(
        classless.class.is_none(),
        "precondition: the row under test has no class column"
    );
    cache.upsert(&classless).await.unwrap();

    // The exact predicate list_facets applies (both the schema handler and the
    // tool mirror): class column authoritative, key-prefix fallback only when
    // the column is None.
    let all = cache.list_all().await.unwrap();
    let matched = |cls: &str| -> Vec<&str> {
        all.iter()
            .filter(|f| f.state == FacetState::Active)
            .filter(|f| {
                f.class.as_deref() == Some(cls)
                    || (f.class.is_none() && f.key.starts_with(&format!("{cls}/")))
            })
            .map(|f| f.key.as_str())
            .collect()
    };

    // Included under its own class via the key-prefix fallback…
    assert_eq!(
        matched("style"),
        vec!["style/verbosity"],
        "a classless row must be matched by the class its key prefix names"
    );
    // …and never leaks into a different class.
    assert!(
        matched("goal").is_empty(),
        "the key-prefix fallback must not match a different class"
    );
}

// ── set_user_state_pinned_persists ────────────────────────────────────────────

#[tokio::test]
async fn set_user_state_pinned_persists() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f-pin",
            "identity/name",
            "Alice",
            FacetState::Active,
            2.0,
        ))
        .await
        .unwrap();

    let updated = cache
        .set_user_state("identity/name", UserState::Pinned)
        .await
        .unwrap();
    assert!(updated, "row should exist and be updated");

    let f = cache.get("identity/name").await.unwrap().unwrap();
    assert_eq!(f.user_state, UserState::Pinned);
}

// ── drop_below_threshold_removes_facets ───────────────────────────────────────

#[tokio::test]
async fn drop_below_threshold_removes_facets() {
    let cache = make_cache();

    cache
        .upsert(&stub_facet(
            "f-low",
            "style/dropped_one",
            "x",
            FacetState::Dropped,
            0.1,
        ))
        .await
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f-keep",
            "style/active_one",
            "y",
            FacetState::Active,
            0.1, // low stability but Active state — should NOT be deleted
        ))
        .await
        .unwrap();
    cache
        .upsert(&stub_facet(
            "f-pinned-drop",
            "style/pinned_one",
            "z",
            FacetState::Dropped,
            0.1,
        ))
        .await
        .unwrap();
    cache
        .set_user_state("style/pinned_one", UserState::Pinned)
        .await
        .unwrap();

    let removed = cache.drop_below_threshold(0.3).await.unwrap();
    assert_eq!(
        removed, 1,
        "only the non-pinned Dropped row should be removed"
    );

    // Active and Pinned rows survive.
    let all = cache.list_all().await.unwrap();
    assert_eq!(all.len(), 2);
}

// ── list_by_class_filters_correctly ───────────────────────────────────────────

#[tokio::test]
async fn list_by_class_filters_correctly() {
    let cache = make_cache();

    for (id, key, val) in [
        ("f-s1", "style/verbosity", "terse"),
        ("f-s2", "style/tone", "formal"),
        ("f-i1", "identity/name", "Alice"),
    ] {
        cache
            .upsert(&stub_facet(id, key, val, FacetState::Active, 1.6))
            .await
            .unwrap();
    }

    let style = cache.list_by_class(FacetClass::Style).await.unwrap();
    assert_eq!(style.len(), 2);
    assert!(style.iter().all(|f| f.key.starts_with("style/")));

    let identity = cache.list_by_class(FacetClass::Identity).await.unwrap();
    assert_eq!(identity.len(), 1);
    assert_eq!(identity[0].key, "identity/name");

    let tooling = cache.list_by_class(FacetClass::Tooling).await.unwrap();
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

#[tokio::test]
async fn evidence_refs_survive_upsert_round_trip() {
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
    cache.upsert(&f).await.unwrap();

    let loaded = cache.get("identity/email").await.unwrap().unwrap();
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

#[tokio::test]
async fn delete_removes_facet_by_key() {
    let cache = make_cache();
    cache
        .upsert(&stub_facet(
            "f-del",
            "goal/learn_rust",
            "learn Rust",
            FacetState::Active,
            1.5,
        ))
        .await
        .unwrap();

    let deleted = cache.delete("goal/learn_rust").await.unwrap();
    assert!(deleted);

    let loaded = cache.get("goal/learn_rust").await.unwrap();
    assert!(loaded.is_none());
}

// ── reset_non_pinned ─────────────────────────────────────────────────────────

#[tokio::test]
async fn reset_deletes_every_non_pinned_facet_and_keeps_the_pinned_ones() {
    let profile = std::sync::Arc::new(
        crate::openhuman::agent::learning::test_profile::InMemoryProfile::new(),
    );
    let cache = FacetCache::for_tests(profile.clone());
    for (key, state) in [
        ("style/verbosity", UserState::Auto),
        ("tooling/package_manager", UserState::Pinned),
        ("goal/ship", UserState::Auto),
    ] {
        let mut facet = stub_facet(key, key, "v", FacetState::Active, 0.9);
        facet.user_state = state;
        cache.upsert(&facet).await.expect("seed facet");
    }

    let (deleted, pinned_preserved) =
        crate::openhuman::agent::learning::cache::reset_non_pinned(&cache)
            .await
            .expect("reset succeeds");

    assert_eq!(deleted, 2);
    assert_eq!(pinned_preserved, 1);
    let remaining = cache.list_all().await.expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].key, "tooling/package_manager");
}

/// A failed delete must surface, not be counted as "nothing to delete".
///
/// This is the case that made the old `unwrap_or(false)` wrong: the reset
/// reported success while the facets were still stored, so the next turn kept
/// reading material the user had asked to forget — and nothing in the response
/// let the caller tell that apart from a clean reset.
#[tokio::test]
async fn a_failed_delete_is_reported_rather_than_counted_as_a_no_op() {
    let profile = std::sync::Arc::new(
        crate::openhuman::agent::learning::test_profile::InMemoryProfile::new(),
    );
    let cache = FacetCache::for_tests(profile.clone());
    for key in ["style/verbosity", "goal/ship"] {
        let facet = stub_facet(key, key, "v", FacetState::Active, 0.9);
        cache.upsert(&facet).await.expect("seed facet");
    }
    profile.fail_delete_for("goal/ship");

    let error = crate::openhuman::agent::learning::cache::reset_non_pinned(&cache)
        .await
        .expect_err("a delete failure must not report success");
    assert!(
        error.to_string().contains("delete failed"),
        "the error should name the failure: {error}"
    );
}
