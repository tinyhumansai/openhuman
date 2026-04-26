use super::*;

fn setup_db() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SEGMENTS_INIT_SQL).unwrap();
    // Also need episodic tables for integration.
    conn.execute_batch(super::super::fts5::EPISODIC_INIT_SQL)
        .unwrap();
    Arc::new(Mutex::new(conn))
}

#[test]
fn create_and_get_segment() {
    let conn = setup_db();
    segment_create(&conn, "seg-1", "s1", "global", 1, 1000.0, 1000.0).unwrap();
    let seg = segment_get(&conn, "seg-1").unwrap().unwrap();
    assert_eq!(seg.session_id, "s1");
    assert_eq!(seg.turn_count, 1);
    assert_eq!(seg.status, SegmentStatus::Open);
}

#[test]
fn append_and_close_segment() {
    let conn = setup_db();
    segment_create(&conn, "seg-2", "s1", "global", 1, 1000.0, 1000.0).unwrap();
    segment_append_turn(&conn, "seg-2", 2, 1005.0, 1005.0).unwrap();
    segment_append_turn(&conn, "seg-2", 3, 1010.0, 1010.0).unwrap();

    let seg = segment_get(&conn, "seg-2").unwrap().unwrap();
    assert_eq!(seg.turn_count, 3);
    assert_eq!(seg.end_episodic_id, Some(3));

    segment_close(&conn, "seg-2", 1010.0).unwrap();
    let seg = segment_get(&conn, "seg-2").unwrap().unwrap();
    assert_eq!(seg.status, SegmentStatus::Closed);
}

#[test]
fn open_segment_for_session_returns_latest() {
    let conn = setup_db();
    segment_create(&conn, "seg-a", "s1", "global", 1, 1000.0, 1000.0).unwrap();
    segment_close(&conn, "seg-a", 1001.0).unwrap();
    segment_create(&conn, "seg-b", "s1", "global", 5, 1010.0, 1010.0).unwrap();

    let open = open_segment_for_session(&conn, "s1").unwrap();
    assert!(open.is_some());
    assert_eq!(open.unwrap().segment_id, "seg-b");

    // Different session has none.
    let none = open_segment_for_session(&conn, "s2").unwrap();
    assert!(none.is_none());
}

#[test]
fn boundary_detection_time_gap() {
    let config = BoundaryConfig::default();
    let seg = ConversationSegment {
        segment_id: "s1".into(),
        session_id: "sess".into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: Some(5),
        start_timestamp: 1000.0,
        end_timestamp: Some(1050.0),
        turn_count: 5,
        summary: None,
        embedding: None,
        topic_keywords: None,
        status: SegmentStatus::Open,
        created_at: 1000.0,
        updated_at: 1050.0,
    };

    // Within time gap — continue.
    let decision = detect_boundary(&config, &seg, 1100.0, "hello", None);
    assert!(matches!(decision, BoundaryDecision::Continue));

    // Exceeds time gap — boundary.
    let decision = detect_boundary(&config, &seg, 1700.0, "hello", None);
    assert!(matches!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::TimeGap)
    ));
}

#[test]
fn boundary_detection_explicit_marker() {
    let config = BoundaryConfig::default();
    let seg = ConversationSegment {
        segment_id: "s1".into(),
        session_id: "sess".into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: None,
        start_timestamp: 1000.0,
        end_timestamp: None,
        turn_count: 2,
        summary: None,
        embedding: None,
        topic_keywords: None,
        status: SegmentStatus::Open,
        created_at: 1000.0,
        updated_at: 1000.0,
    };

    let decision = detect_boundary(
        &config,
        &seg,
        1005.0,
        "Switching to a different topic",
        None,
    );
    assert!(matches!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::ExplicitMarker)
    ));
}

#[test]
fn boundary_detection_turn_count() {
    let config = BoundaryConfig {
        max_turns_per_segment: 5,
        ..Default::default()
    };
    let seg = ConversationSegment {
        segment_id: "s1".into(),
        session_id: "sess".into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: Some(5),
        start_timestamp: 1000.0,
        end_timestamp: Some(1010.0),
        turn_count: 5,
        summary: None,
        embedding: None,
        topic_keywords: None,
        status: SegmentStatus::Open,
        created_at: 1000.0,
        updated_at: 1010.0,
    };

    let decision = detect_boundary(&config, &seg, 1011.0, "next", None);
    assert!(matches!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::TurnCountExceeded)
    ));
}

#[test]
fn boundary_detection_embedding_drift() {
    let config = BoundaryConfig::default();
    let seg = ConversationSegment {
        segment_id: "s1".into(),
        session_id: "sess".into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: None,
        start_timestamp: 1000.0,
        end_timestamp: None,
        turn_count: 3,
        summary: None,
        embedding: Some(vec![1.0, 0.0, 0.0]),
        topic_keywords: None,
        status: SegmentStatus::Open,
        created_at: 1000.0,
        updated_at: 1000.0,
    };

    // Similar direction — continue.
    let decision = detect_boundary(&config, &seg, 1005.0, "hello", Some(&[0.9, 0.1, 0.0]));
    assert!(matches!(decision, BoundaryDecision::Continue));

    // Orthogonal direction — boundary.
    let decision = detect_boundary(&config, &seg, 1005.0, "hello", Some(&[0.0, 1.0, 0.0]));
    assert!(matches!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::EmbeddingDrift)
    ));
}

#[test]
fn incremental_mean_embedding_works() {
    let centroid = vec![1.0, 0.0];
    let new = vec![0.0, 1.0];
    let result = incremental_mean_embedding(&centroid, &new, 1);
    // After 2 vectors: mean should be [0.5, 0.5]
    assert!((result[0] - 0.5).abs() < 0.01);
    assert!((result[1] - 0.5).abs() < 0.01);
}

#[test]
fn summary_set_and_read() {
    let conn = setup_db();
    segment_create(&conn, "seg-s", "s1", "global", 1, 1000.0, 1000.0).unwrap();
    segment_close(&conn, "seg-s", 1001.0).unwrap();
    segment_set_summary(&conn, "seg-s", "Discussed deployment strategy", 1002.0).unwrap();
    let seg = segment_get(&conn, "seg-s").unwrap().unwrap();
    assert_eq!(seg.status, SegmentStatus::Summarised);
    assert_eq!(
        seg.summary.as_deref(),
        Some("Discussed deployment strategy")
    );
}

#[test]
fn segments_by_namespace_returns_most_recent_first() {
    let conn = setup_db();
    // Create three segments with different updated_at timestamps.
    segment_create(&conn, "seg-ns-1", "s1", "myns", 1, 1000.0, 1000.0).unwrap();
    segment_create(&conn, "seg-ns-2", "s1", "myns", 5, 2000.0, 2000.0).unwrap();
    segment_create(&conn, "seg-ns-3", "s1", "myns", 10, 3000.0, 3000.0).unwrap();

    // Append a turn to seg-ns-1 with a later timestamp to bump its updated_at.
    // Leave seg-ns-3 as the most recently created (highest updated_at).
    let segs = segments_by_namespace(&conn, "myns", 10).unwrap();
    assert_eq!(segs.len(), 3, "Expected 3 segments in namespace");

    // Most recently updated segment should come first (DESC order on updated_at).
    assert_eq!(segs[0].segment_id, "seg-ns-3");
    assert_eq!(segs[1].segment_id, "seg-ns-2");
    assert_eq!(segs[2].segment_id, "seg-ns-1");

    // Bump seg-ns-1's updated_at by appending a turn.
    segment_append_turn(&conn, "seg-ns-1", 2, 9000.0, 9000.0).unwrap();
    let segs = segments_by_namespace(&conn, "myns", 10).unwrap();
    assert_eq!(segs[0].segment_id, "seg-ns-1");
}

#[test]
fn segments_pending_summary_only_returns_closed() {
    let conn = setup_db();
    // Open segment — should NOT appear.
    segment_create(&conn, "seg-open", "s1", "global", 1, 1000.0, 1000.0).unwrap();

    // Closed segment — SHOULD appear.
    segment_create(&conn, "seg-closed", "s2", "global", 5, 2000.0, 2000.0).unwrap();
    segment_close(&conn, "seg-closed", 2001.0).unwrap();

    // Summarised segment — should NOT appear (only status='closed' is pending).
    segment_create(&conn, "seg-summ", "s3", "global", 10, 3000.0, 3000.0).unwrap();
    segment_close(&conn, "seg-summ", 3001.0).unwrap();
    segment_set_summary(&conn, "seg-summ", "A summary", 3002.0).unwrap();

    let pending = segments_pending_summary(&conn, 20).unwrap();
    assert_eq!(
        pending.len(),
        1,
        "Only the closed segment should be pending"
    );
    assert_eq!(pending[0].segment_id, "seg-closed");
    assert_eq!(pending[0].status, SegmentStatus::Closed);
}

#[test]
fn segment_set_embedding_roundtrip() {
    let conn = setup_db();
    segment_create(&conn, "seg-emb", "s1", "global", 1, 1000.0, 1000.0).unwrap();

    let embedding = vec![0.1_f32, 0.2, 0.3, 0.4, 0.5];
    segment_set_embedding(&conn, "seg-emb", &embedding, 1001.0).unwrap();

    let seg = segment_get(&conn, "seg-emb").unwrap().unwrap();
    let stored = seg.embedding.expect("embedding should be stored");
    assert_eq!(stored.len(), embedding.len());
    for (stored_val, expected_val) in stored.iter().zip(embedding.iter()) {
        assert!(
            (stored_val - expected_val).abs() < 1e-6,
            "Embedding value mismatch: got {stored_val}, expected {expected_val}"
        );
    }
}

#[test]
fn segment_set_keywords_stores_and_reads() {
    let conn = setup_db();
    segment_create(&conn, "seg-kw", "s1", "global", 1, 1000.0, 1000.0).unwrap();

    let keywords = "rust,memory,performance";
    segment_set_keywords(&conn, "seg-kw", keywords, 1001.0).unwrap();

    let seg = segment_get(&conn, "seg-kw").unwrap().unwrap();
    assert_eq!(
        seg.topic_keywords.as_deref(),
        Some("rust,memory,performance"),
        "Keywords should round-trip correctly"
    );
}

#[test]
fn boundary_no_false_positive_on_short_messages() {
    let config = BoundaryConfig::default();
    let seg = ConversationSegment {
        segment_id: "s1".into(),
        session_id: "sess".into(),
        namespace: "global".into(),
        start_episodic_id: 1,
        end_episodic_id: Some(3),
        start_timestamp: 1000.0,
        end_timestamp: Some(1010.0),
        turn_count: 3,
        summary: None,
        embedding: None,
        topic_keywords: None,
        status: SegmentStatus::Open,
        created_at: 1000.0,
        updated_at: 1010.0,
    };

    // Short single-word messages must not trigger explicit marker detection.
    for short_msg in &["yes", "ok", "no", "sure", "thanks", "great"] {
        let decision = detect_boundary(&config, &seg, 1011.0, short_msg, None);
        assert!(
            matches!(decision, BoundaryDecision::Continue),
            "Short message '{short_msg}' incorrectly triggered a boundary"
        );
    }
}

#[test]
fn fallback_summary_truncates_long_content() {
    let long = "a".repeat(300);
    let short = "brief ending";
    let summary = fallback_summary(&long, short, 5);

    // The truncated first content should end with "..." and be capped at 203 chars
    // (200 chars + "...").
    assert!(
        summary.contains("..."),
        "Long content should be truncated with ellipsis"
    );
    assert!(
        !summary.contains(&long),
        "Full long content should not appear verbatim in summary"
    );
    // The summary should still reference the short last content.
    assert!(
        summary.contains(short),
        "Last content should appear in summary"
    );
    // Verify exact truncation: first 200 chars of `long` followed by "...".
    let truncated_first = format!("{}...", &long[..200]);
    assert!(summary.contains(&truncated_first));
}
