use super::*;
use crate::openhuman::agent::learning::candidate::{
    Buffer, EvidenceRef, FacetClass, LearningCandidate,
};

fn make_detector() -> StabilityDetector {
    let cache = crate::openhuman::agent::learning::test_profile::in_memory_cache();
    // Use a private buffer so tests don't interfere with the global singleton.
    let buffer: &'static Buffer = Box::leak(Box::new(Buffer::new(256)));
    StabilityDetector { cache, buffer }
}

fn make_candidate(
    class: FacetClass,
    key: &str,
    value: &str,
    cue: CueFamily,
    observed_at: f64,
) -> LearningCandidate {
    LearningCandidate {
        class,
        key: key.into(),
        value: value.into(),
        cue_family: cue,
        evidence: EvidenceRef::Episodic { episodic_id: 1 },
        initial_confidence: 0.8,
        observed_at,
    }
}

// ── stability formula ────────────────────────────────────────────────────

#[test]
fn stability_pinned_returns_infinity() {
    let s = stability(
        CueFamily::Behavioral,
        5,
        0.0,
        1000.0,
        FacetClass::Style,
        false,
        UserState::Pinned,
    );
    assert!(s.is_infinite() && s > 0.0);
}

#[test]
fn stability_forgotten_returns_zero() {
    let s = stability(
        CueFamily::Explicit,
        100,
        0.0,
        1000.0,
        FacetClass::Style,
        true,
        UserState::Forgotten,
    );
    assert_eq!(s, 0.0);
}

#[test]
fn stability_explicit_doubles_score() {
    let base = stability(
        CueFamily::Explicit,
        3,
        1_000_000.0,
        1_000_001.0,
        FacetClass::Style,
        false, // no_explicit
        UserState::Auto,
    );
    let with_explicit = stability(
        CueFamily::Explicit,
        3,
        1_000_000.0,
        1_000_001.0,
        FacetClass::Style,
        true, // has_explicit
        UserState::Auto,
    );
    assert!(
        (with_explicit - 2.0 * base).abs() < 1e-9,
        "explicit multiplier must be exactly 2x: base={base:.6} explicit={with_explicit:.6}"
    );
}

#[test]
fn stability_decays_over_time() {
    let now = 1_000_000.0_f64;
    let recent = stability(
        CueFamily::Behavioral,
        5,
        now - 100.0, // observed 100 s ago
        now,
        FacetClass::Style,
        false,
        UserState::Auto,
    );
    let old = stability(
        CueFamily::Behavioral,
        5,
        now - HALF_LIFE_STYLE, // observed one half-life ago
        now,
        FacetClass::Style,
        false,
        UserState::Auto,
    );
    assert!(
        recent > old,
        "recent evidence should produce higher stability: recent={recent:.4} old={old:.4}"
    );
    // At exactly one half-life, recency = exp(-1) ≈ 0.368.
    assert!(
        old / recent < 0.4,
        "decay over one half-life should be substantial: ratio={}",
        old / recent
    );
}

// ── rebuild ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn rebuild_empty_buffer_no_candidates_is_noop() {
    let detector = make_detector();
    let now = 1_000_000.0;
    // No candidates, no existing rows → rebuild is a no-op.
    let outcome = detector.rebuild(now).await.unwrap();
    assert_eq!(outcome.added, 0);
    assert_eq!(outcome.evicted, 0);
    assert_eq!(outcome.kept, 0);
    assert_eq!(outcome.total_size, 0);
}

#[tokio::test]
async fn rebuild_strong_candidate_becomes_active() {
    let detector = make_detector();
    let now = 1_000_000.0;

    // Push enough explicit evidence to clear τ_promote.
    for i in 0..5 {
        detector.buffer.push(make_candidate(
            FacetClass::Style,
            "verbosity",
            "terse",
            CueFamily::Explicit,
            now - i as f64 * 10.0,
        ));
    }

    let outcome = detector.rebuild(now).await.unwrap();
    assert_eq!(outcome.added, 1);

    let actives = detector.cache.list_active().await.unwrap();
    assert_eq!(actives.len(), 1);
    assert_eq!(actives[0].key, "style/verbosity");
    assert_eq!(actives[0].value, "terse");
    assert_eq!(actives[0].state, FacetState::Active);
}

#[tokio::test]
async fn rebuild_conflict_resolution_picks_stronger_value() {
    let detector = make_detector();
    let now = 1_000_000.0;

    // 3 explicit candidates for "terse", 1 behavioral for "verbose".
    for _ in 0..3 {
        detector.buffer.push(make_candidate(
            FacetClass::Style,
            "verbosity",
            "terse",
            CueFamily::Explicit,
            now - 10.0,
        ));
    }
    detector.buffer.push(make_candidate(
        FacetClass::Style,
        "verbosity",
        "verbose",
        CueFamily::Behavioral,
        now - 5.0,
    ));

    detector.rebuild(now).await.unwrap();
    let actives = detector.cache.list_active().await.unwrap();
    assert!(!actives.is_empty(), "should have at least one active row");
    let verbosity = actives.iter().find(|f| f.key == "style/verbosity").unwrap();
    assert_eq!(
        verbosity.value, "terse",
        "terse had stronger evidence and should win"
    );
}

#[tokio::test]
async fn rebuild_class_budget_respected() {
    let detector = make_detector();
    let now = 1_000_000.0;

    // Push 6 different style keys — budget is BUDGET_STYLE = 4.
    for i in 0..6 {
        let key = format!("style_key_{i}");
        // Push several candidates per key so they clear τ_promote.
        for j in 0..5 {
            detector.buffer.push(LearningCandidate {
                class: FacetClass::Style,
                key: key.clone(),
                value: "v".into(),
                cue_family: CueFamily::Explicit,
                evidence: EvidenceRef::Episodic {
                    episodic_id: i * 10 + j,
                },
                initial_confidence: 0.9,
                observed_at: now - j as f64,
            });
        }
    }

    detector.rebuild(now).await.unwrap();

    let by_class = detector
        .cache
        .list_by_class(FacetClass::Style)
        .await
        .unwrap();
    assert!(
        by_class.len() <= BUDGET_STYLE,
        "style class should have at most {BUDGET_STYLE} active rows, got {}",
        by_class.len()
    );
}

#[tokio::test]
async fn rebuild_pinned_facet_stays_active_regardless_of_stability() {
    let detector = make_detector();
    let now = 1_000_000.0;

    // Manually insert a Pinned row.
    use tinymemory_api::provider::{FacetState, FacetType, UserState};
    let pinned = ProfileFacet {
        facet_id: "f-pinned".into(),
        facet_type: FacetType::Preference,
        key: "style/format".into(),
        value: "markdown".into(),
        confidence: 0.9,
        evidence_count: 1,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1000.0, // very old — would normally decay
        state: FacetState::Active,
        stability: 0.0,
        user_state: UserState::Pinned,
        evidence_refs: vec![],
        class: Some("style".into()),
        cue_families: None,
    };
    detector.cache.upsert(&pinned).await.unwrap();

    // No new candidates for this key → only decay applies.
    detector.rebuild(now).await.unwrap();

    let f = detector
        .cache
        .get("style/format")
        .await
        .unwrap()
        .expect("pinned row must survive");
    assert_eq!(f.state, FacetState::Active);
}

// ── half_life ────────────────────────────────────────────────────────────

#[test]
fn half_life_ordering_matches_spec() {
    // Identity decays slowest; Channel decays fastest.
    assert!(half_life(FacetClass::Identity) > half_life(FacetClass::Veto));
    assert!(half_life(FacetClass::Veto) > half_life(FacetClass::Tooling));
    assert!(half_life(FacetClass::Tooling) >= half_life(FacetClass::Goal));
    assert!(half_life(FacetClass::Goal) > half_life(FacetClass::Style));
    assert!(half_life(FacetClass::Style) > half_life(FacetClass::Channel));
}

// ── class_budget ────────────────────────────────────────────────────────

#[test]
fn class_budget_values_match_spec() {
    assert_eq!(class_budget(FacetClass::Style), 4);
    assert_eq!(class_budget(FacetClass::Identity), 4);
    assert_eq!(class_budget(FacetClass::Tooling), 5);
    assert_eq!(class_budget(FacetClass::Veto), 3);
    assert_eq!(class_budget(FacetClass::Goal), 3);
    assert_eq!(class_budget(FacetClass::Channel), 1);
}

// ── most_recent_reinforcement floor ───────────────────────────────────────

#[test]
fn reinforcement_floor_scopes_to_facet_class() {
    // With no candidates and no existing row, the result is purely the
    // class-scoped floor `now - half_life(class)`. This pins that the floor
    // tracks the facet's own class rather than a hardcoded one — the longer
    // half-lives (Goal, Identity) must floor further in the past than Style,
    // and Channel (shortest) closer to now.
    let now = 10_000_000.0;
    for class in [
        FacetClass::Identity,
        FacetClass::Veto,
        FacetClass::Tooling,
        FacetClass::Goal,
        FacetClass::Style,
        FacetClass::Channel,
    ] {
        let floor = most_recent_reinforcement(&[], None, now, class);
        assert_eq!(
            floor,
            now - half_life(class),
            "floor must use {class:?}'s own half-life"
        );
    }
    // Guard against a regression to a single hardcoded class: a class with a
    // different half-life than Style must produce a different floor.
    assert_ne!(
        most_recent_reinforcement(&[], None, now, FacetClass::Goal),
        most_recent_reinforcement(&[], None, now, FacetClass::Style),
    );
}

// ── merge_evidence_refs deduplication ─────────────────────────────────────

#[test]
fn merge_evidence_refs_removes_non_consecutive_duplicates() {
    // The bug this guards: a ref already in the existing row (Episodic 1)
    // that is re-emitted by a new candidate lands non-adjacent to its twin
    // once the two lists are concatenated ([1, 2, 1]). `Vec::dedup_by` only
    // collapses *consecutive* equals, so it would leave the duplicate in and
    // the refs list would grow every rebuild cycle. The set-based merge must
    // drop it, keeping the first occurrence and preserving order.
    let existing = vec![EvidenceRef::Episodic { episodic_id: 1 }];
    let new = vec![
        EvidenceRef::Episodic { episodic_id: 2 },
        EvidenceRef::Episodic { episodic_id: 1 },
    ];
    let merged = merge_evidence_refs(&existing, new);
    assert_eq!(
        merged,
        vec![
            EvidenceRef::Episodic { episodic_id: 1 },
            EvidenceRef::Episodic { episodic_id: 2 },
        ],
        "non-consecutive duplicate must be removed, first-seen order preserved"
    );
}

#[test]
fn merge_evidence_refs_dedups_within_a_single_cycle() {
    // Two candidates in the same cycle can reference the same evidence with
    // an unrelated ref between them; that also defeats consecutive-only dedup.
    let new = vec![
        EvidenceRef::TreeTopic {
            topic_id: "a".into(),
        },
        EvidenceRef::Episodic { episodic_id: 7 },
        EvidenceRef::TreeTopic {
            topic_id: "a".into(),
        },
    ];
    let merged = merge_evidence_refs(&[], new);
    assert_eq!(
        merged,
        vec![
            EvidenceRef::TreeTopic {
                topic_id: "a".into(),
            },
            EvidenceRef::Episodic { episodic_id: 7 },
        ],
    );
}

#[test]
fn merge_evidence_refs_is_idempotent_across_rebuilds() {
    // Re-running with the merged result as the new existing row and the same
    // candidates must not grow the list — the core invariant that the old
    // consecutive-only dedup violated.
    let existing = vec![
        EvidenceRef::Episodic { episodic_id: 1 },
        EvidenceRef::Episodic { episodic_id: 2 },
    ];
    let cands = vec![
        EvidenceRef::Episodic { episodic_id: 2 },
        EvidenceRef::Episodic { episodic_id: 1 },
    ];
    let first = merge_evidence_refs(&existing, cands.clone());
    let second = merge_evidence_refs(&first, cands);
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
}
