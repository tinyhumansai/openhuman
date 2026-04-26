use super::*;

fn setup_db() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(EVENTS_INIT_SQL).unwrap();
    Arc::new(Mutex::new(conn))
}

#[test]
fn insert_and_search_event() {
    let conn = setup_db();
    let event = EventRecord {
        event_id: "evt-1".into(),
        segment_id: "seg-1".into(),
        session_id: "s1".into(),
        namespace: "global".into(),
        event_type: EventType::Decision,
        content: "We decided to use Rust for the backend".into(),
        subject: Some("backend language".into()),
        timestamp_ref: None,
        confidence: 0.8,
        embedding: None,
        source_turn_ids: None,
        created_at: 1000.0,
    };
    event_insert(&conn, &event).unwrap();

    let results = event_search_fts(&conn, "global", "Rust backend", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_type, EventType::Decision);
}

#[test]
fn heuristic_extraction_finds_patterns() {
    let text = "I prefer dark mode for coding. We decided to use PostgreSQL. \
                 The deadline is by Friday. I live in Berlin. \
                 This is a regular sentence with no pattern.";
    let events = extract_events_heuristic(text);

    let types: Vec<&EventType> = events.iter().map(|(t, _)| t).collect();
    assert!(types.contains(&&EventType::Preference));
    assert!(types.contains(&&EventType::Decision));
    assert!(types.contains(&&EventType::Commitment));
    assert!(types.contains(&&EventType::Fact));
    // Regular sentence should NOT be extracted.
    assert!(!events.iter().any(|(_, s)| s.contains("regular sentence")));
}

#[test]
fn events_for_segment_returns_ordered() {
    let conn = setup_db();
    for i in 0..3 {
        event_insert(
            &conn,
            &EventRecord {
                event_id: format!("evt-{i}"),
                segment_id: "seg-1".into(),
                session_id: "s1".into(),
                namespace: "global".into(),
                event_type: EventType::Fact,
                content: format!("Fact number {i}"),
                subject: None,
                timestamp_ref: None,
                confidence: 0.7,
                embedding: None,
                source_turn_ids: None,
                created_at: 1000.0 + i as f64,
            },
        )
        .unwrap();
    }

    let events = events_for_segment(&conn, "seg-1").unwrap();
    assert_eq!(events.len(), 3);
    assert!(events[0].created_at < events[2].created_at);
}

#[test]
fn event_insert_idempotent() {
    let conn = setup_db();
    let event = EventRecord {
        event_id: "evt-idem".into(),
        segment_id: "seg-1".into(),
        session_id: "s1".into(),
        namespace: "global".into(),
        event_type: EventType::Fact,
        content: "Rust is a systems language".into(),
        subject: None,
        timestamp_ref: None,
        confidence: 0.9,
        embedding: None,
        source_turn_ids: None,
        created_at: 1000.0,
    };
    // Insert same event_id twice — OR REPLACE semantics; no duplicate row.
    event_insert(&conn, &event).unwrap();
    event_insert(&conn, &event).unwrap();

    let events = events_for_segment(&conn, "seg-1").unwrap();
    assert_eq!(
        events.len(),
        1,
        "Duplicate insert should not create a second row"
    );
}

#[test]
fn events_by_type_filters_correctly() {
    let conn = setup_db();

    let make_event = |id: &str, event_type: EventType, ns: &str| EventRecord {
        event_id: id.to_string(),
        segment_id: "seg-x".into(),
        session_id: "s1".into(),
        namespace: ns.to_string(),
        event_type,
        content: format!("Content for {id}"),
        subject: None,
        timestamp_ref: None,
        confidence: 0.7,
        embedding: None,
        source_turn_ids: None,
        created_at: 1000.0,
    };

    event_insert(&conn, &make_event("e-dec", EventType::Decision, "ns1")).unwrap();
    event_insert(&conn, &make_event("e-pref", EventType::Preference, "ns1")).unwrap();
    event_insert(&conn, &make_event("e-fact", EventType::Fact, "ns1")).unwrap();

    let decisions = events_by_type(&conn, "ns1", "decision", 10).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].event_id, "e-dec");
    assert_eq!(decisions[0].event_type, EventType::Decision);

    let prefs = events_by_type(&conn, "ns1", "preference", 10).unwrap();
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].event_id, "e-pref");

    // Different namespace should return nothing.
    let other = events_by_type(&conn, "ns2", "decision", 10).unwrap();
    assert!(
        other.is_empty(),
        "No events expected for unrelated namespace"
    );
}

#[test]
fn heuristic_extracts_multiple_from_same_sentence() {
    // A sentence that simultaneously satisfies a preference pattern AND a fact
    // pattern will only produce one event (dedup guard). Use two separate
    // sentences to confirm both types are emitted.
    let text = "I prefer Python for scripting. I live in Berlin.";
    let events = extract_events_heuristic(text);

    let types: Vec<&EventType> = events.iter().map(|(t, _)| t).collect();
    assert!(
        types.contains(&&EventType::Preference),
        "Expected a Preference event from 'I prefer Python'"
    );
    assert!(
        types.contains(&&EventType::Fact),
        "Expected a Fact event from 'I live in Berlin'"
    );
    assert!(
        events.len() >= 2,
        "Expected at least 2 events, got {}",
        events.len()
    );
}

#[test]
fn heuristic_handles_empty_and_whitespace() {
    assert!(
        extract_events_heuristic("").is_empty(),
        "Empty string should yield no events"
    );
    assert!(
        extract_events_heuristic("   \n\t  ").is_empty(),
        "Whitespace-only string should yield no events"
    );
}

#[test]
fn event_fts_matches_subject_field() {
    let conn = setup_db();
    let event = EventRecord {
        event_id: "evt-subj".into(),
        segment_id: "seg-1".into(),
        session_id: "s1".into(),
        namespace: "global".into(),
        event_type: EventType::Decision,
        content: "We agreed on the final design".into(),
        subject: Some("microservice architecture".into()),
        timestamp_ref: None,
        confidence: 0.85,
        embedding: None,
        source_turn_ids: None,
        created_at: 1000.0,
    };
    event_insert(&conn, &event).unwrap();

    // Search by content (should match).
    let by_content = event_search_fts(&conn, "global", "design", 5).unwrap();
    assert_eq!(by_content.len(), 1, "FTS should match on content field");

    // Search by subject text (should also match via event_fts).
    let by_subject = event_search_fts(&conn, "global", "microservice", 5).unwrap();
    assert_eq!(by_subject.len(), 1, "FTS should match on subject field");
    assert_eq!(by_subject[0].event_id, "evt-subj");
}
