use super::*;

fn prov() -> Provenance {
    Provenance {
        thread_id: Some("thr_abc".into()),
        transcript_path: "/tmp/session_raw/123_main.jsonl".into(),
        transcript_basename: "123_main.jsonl".into(),
        extracted_at: "2026-05-09T12:00:00Z".into(),
    }
}

#[test]
fn skips_short_filler() {
    assert!(is_filler("ok"));
    assert!(is_filler("thanks!"));
    assert!(is_filler("hi"));
    assert!(!is_filler("I prefer Postgres for this kind of thing."));
}

#[test]
fn extracts_user_preference_as_high() {
    let msgs = vec![ChatMessage::user(
        "I prefer Postgres over MySQL for new services.",
    )];
    let cands = extract_candidates(&msgs, &prov());
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].kind, CandidateKind::Preference);
    assert_eq!(cands[0].importance, Importance::High);
    assert!(cands[0].content.contains("Postgres"));
}

#[test]
fn does_not_extract_preference_from_assistant() {
    let msgs = vec![ChatMessage::assistant(
        "You said earlier that I prefer Postgres for these.",
    )];
    let cands = extract_candidates(&msgs, &prov());
    assert!(
        cands.iter().all(|c| c.kind != CandidateKind::Preference),
        "assistant text must not produce Preference: {:?}",
        cands,
    );
}

#[test]
fn extracts_decision_from_either_side() {
    let msgs = vec![
        ChatMessage::user("Let's go with Postgres for the metadata store."),
        ChatMessage::assistant("Sure, going with Postgres."),
    ];
    let cands = extract_candidates(&msgs, &prov());
    let decisions: Vec<_> = cands
        .iter()
        .filter(|c| c.kind == CandidateKind::Decision)
        .collect();
    assert!(
        !decisions.is_empty(),
        "should extract at least one decision"
    );
}

#[test]
fn extracts_unresolved_task() {
    let msgs = vec![ChatMessage::user(
        "Still need to migrate the old auth service before Friday.",
    )];
    let cands = extract_candidates(&msgs, &prov());
    assert!(cands
        .iter()
        .any(|c| c.kind == CandidateKind::UnresolvedTask));
}

#[test]
fn captures_reflection_with_provenance_indices() {
    let msgs = vec![
        ChatMessage::user("Hello, can you help with the deploy?"),
        ChatMessage::assistant("Sure, what's broken?"),
        ChatMessage::user(
            "I realized our staging cluster is the bottleneck — \
             next time let's pre-warm it.",
        ),
    ];
    let refls = extract_reflections(&msgs, &prov());
    assert_eq!(refls.len(), 1);
    assert_eq!(refls[0].theme, "user_reflection");
    assert_eq!(refls[0].provenance.message_indices, vec![2]);
}
