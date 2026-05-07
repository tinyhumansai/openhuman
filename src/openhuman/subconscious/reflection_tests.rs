//! Unit tests for `reflection.rs` — wire shape, hydration, dedup, cap.

use super::*;

#[test]
fn reflection_kind_round_trip() {
    for k in [
        ReflectionKind::HotnessSpike,
        ReflectionKind::CrossSourcePattern,
        ReflectionKind::DailyDigest,
        ReflectionKind::DueItem,
        ReflectionKind::Risk,
        ReflectionKind::Opportunity,
    ] {
        assert_eq!(ReflectionKind::from_str_lossy(k.as_str()), k);
    }
    // Unknown -> DailyDigest (most generic).
    assert_eq!(
        ReflectionKind::from_str_lossy("nope"),
        ReflectionKind::DailyDigest
    );
}

#[test]
fn parses_reflection_draft_from_llm_json() {
    // The legacy `disposition` field is silently ignored by serde — kept
    // in the fixture to verify forward/backward compat with LLM responses
    // emitted before the field was dropped from the prompt contract.
    let json = r#"{
        "kind": "hotness_spike",
        "body": "Phoenix has been mentioned 4x in the last hour.",
        "disposition": "notify",
        "proposed_action": "Pull recent Phoenix mentions",
        "source_refs": ["entity:phoenix", "summary:abc"]
    }"#;
    let d: ReflectionDraft = serde_json::from_str(json).expect("parse");
    assert_eq!(d.kind, ReflectionKind::HotnessSpike);
    assert_eq!(
        d.proposed_action.as_deref(),
        Some("Pull recent Phoenix mentions")
    );
    assert_eq!(d.source_refs.len(), 2);
}

#[test]
fn parses_minimal_reflection_draft_without_optional_fields() {
    let json = r#"{
        "kind": "daily_digest",
        "body": "New daily digest sealed."
    }"#;
    let d: ReflectionDraft = serde_json::from_str(json).expect("parse");
    assert!(d.proposed_action.is_none());
    assert!(d.source_refs.is_empty());
}

#[test]
fn hydrate_draft_fills_lifecycle_fields() {
    let draft = ReflectionDraft {
        kind: ReflectionKind::Opportunity,
        body: "User mentioned founders dinner".into(),
        proposed_action: Some("Draft an invite list".into()),
        source_refs: vec!["entity:dinner".into()],
    };
    let r = hydrate_draft(draft, "abc-123".into(), 1_700_000_000.0, Vec::new());
    assert_eq!(r.id, "abc-123");
    assert_eq!(r.created_at, 1_700_000_000.0);
    assert!(r.acted_on_at.is_none());
    assert!(r.dismissed_at.is_none());
}

#[test]
fn dedup_key_is_stable_across_source_ref_order() {
    let body = "Same observation";
    let k1 = dedup_key(
        ReflectionKind::Risk,
        &["a".into(), "b".into(), "c".into()],
        body,
    );
    let k2 = dedup_key(
        ReflectionKind::Risk,
        &["c".into(), "a".into(), "b".into()],
        body,
    );
    assert_eq!(k1, k2);
}

#[test]
fn dedup_key_changes_when_kind_changes() {
    let refs = vec!["a".to_string()];
    let r1 = dedup_key(ReflectionKind::Risk, &refs, "body");
    let r2 = dedup_key(ReflectionKind::Opportunity, &refs, "body");
    assert_ne!(r1, r2);
}

#[test]
fn apply_cap_keeps_within_limit() {
    let drafts: Vec<ReflectionDraft> = (0..3)
        .map(|i| ReflectionDraft {
            kind: ReflectionKind::DailyDigest,
            body: format!("body {i}"),
            proposed_action: None,
            source_refs: vec![],
        })
        .collect();
    let (kept, dropped) = apply_cap(drafts);
    assert_eq!(kept.len(), 3);
    assert_eq!(dropped, 0);
}

#[test]
fn apply_cap_trims_excess() {
    let drafts: Vec<ReflectionDraft> = (0..10)
        .map(|i| ReflectionDraft {
            kind: ReflectionKind::DailyDigest,
            body: format!("body {i}"),
            proposed_action: None,
            source_refs: vec![],
        })
        .collect();
    let (kept, dropped) = apply_cap(drafts);
    assert_eq!(kept.len(), MAX_REFLECTIONS_PER_TICK);
    assert_eq!(dropped, 10 - MAX_REFLECTIONS_PER_TICK);
    assert_eq!(kept[0].body, "body 0"); // FIFO order preserved
}
