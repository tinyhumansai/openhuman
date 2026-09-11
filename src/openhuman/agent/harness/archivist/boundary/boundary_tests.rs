//! Tests for segment-boundary detection and the fallback summary.
//!
//! These moved with the functions. The engine's own tests for them stay where
//! they are until the engine drops the code; until then both suites assert the
//! same behaviour, which is what makes the move checkable.

use super::*;

fn segment(turn_count: i32, start: f64, end: Option<f64>) -> SegmentBoundaryState {
    SegmentBoundaryState {
        turn_count,
        start_timestamp: start,
        end_timestamp: end,
        embedding: None,
    }
}

#[test]
fn a_turn_inside_every_threshold_continues_the_segment() {
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &segment(3, 1000.0, Some(1010.0)),
        1020.0,
        "and what about the error handling?",
        None,
    );
    assert_eq!(decision, BoundaryDecision::Continue);
}

#[test]
fn the_turn_cap_is_checked_before_anything_else() {
    // Within the time gap and with no marker: only the cap can trip here.
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &segment(20, 1000.0, Some(1010.0)),
        1011.0,
        "carry on",
        None,
    );
    assert_eq!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::TurnCountExceeded)
    );
}

#[test]
fn a_long_pause_starts_a_new_segment() {
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &segment(2, 1000.0, Some(1010.0)),
        // 601s after the last turn — one second past the ten-minute gap.
        1611.0,
        "carry on",
        None,
    );
    assert_eq!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::TimeGap)
    );
}

#[test]
fn the_gap_is_measured_from_the_segment_start_when_no_turn_has_landed_yet() {
    // `end_timestamp` is None, so a segment that has only its opening turn is
    // measured from its own start rather than from zero — which would make
    // every second turn look like a ten-minute pause.
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &segment(1, 1000.0, None),
        1100.0,
        "carry on",
        None,
    );
    assert_eq!(decision, BoundaryDecision::Continue);
}

#[test]
fn a_topic_change_marker_starts_a_new_segment_case_insensitively() {
    for content in [
        "BTW, what time is it?",
        "Anyway, moving on",
        "By The Way, one more thing",
    ] {
        let decision = detect_boundary(
            &BoundaryConfig::default(),
            &segment(2, 1000.0, Some(1010.0)),
            1020.0,
            content,
            None,
        );
        assert_eq!(
            decision,
            BoundaryDecision::Boundary(BoundaryReason::ExplicitMarker),
            "expected {content:?} to read as a topic change"
        );
    }
}

#[test]
fn embedding_drift_below_the_floor_starts_a_new_segment() {
    let mut current = segment(2, 1000.0, Some(1010.0));
    current.embedding = Some(vec![1.0, 0.0]);
    // Orthogonal ⇒ similarity 0.0, below the 0.4 floor.
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &current,
        1020.0,
        "carry on",
        Some(&[0.0, 1.0]),
    );
    assert_eq!(
        decision,
        BoundaryDecision::Boundary(BoundaryReason::EmbeddingDrift)
    );
}

#[test]
fn mismatched_embedding_dimensions_are_skipped_rather_than_read_as_drift() {
    // Two embedding spaces would produce a meaningless similarity; treating
    // that as drift would split segments at random whenever the model changed.
    let mut current = segment(2, 1000.0, Some(1010.0));
    current.embedding = Some(vec![1.0, 0.0, 0.0]);
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &current,
        1020.0,
        "carry on",
        Some(&[0.0, 1.0]),
    );
    assert_eq!(decision, BoundaryDecision::Continue);
}

#[test]
fn an_empty_segment_centroid_is_skipped() {
    let mut current = segment(2, 1000.0, Some(1010.0));
    current.embedding = Some(Vec::new());
    let decision = detect_boundary(
        &BoundaryConfig::default(),
        &current,
        1020.0,
        "carry on",
        Some(&[0.0, 1.0]),
    );
    assert_eq!(decision, BoundaryDecision::Continue);
}

#[test]
fn the_first_vector_becomes_the_centroid_when_there_is_none() {
    assert_eq!(
        incremental_mean_embedding(&[], &[1.0, 2.0], 0),
        vec![1.0, 2.0]
    );
    // Dimension mismatch takes the same escape hatch.
    assert_eq!(
        incremental_mean_embedding(&[1.0], &[1.0, 2.0], 3),
        vec![1.0, 2.0]
    );
}

#[test]
fn the_centroid_moves_toward_the_new_vector_by_one_over_count_plus_one() {
    // count = 1 ⇒ the new vector gets half the weight.
    assert_eq!(
        incremental_mean_embedding(&[0.0, 0.0], &[1.0, 1.0], 1),
        vec![0.5, 0.5]
    );
    // count = 3 ⇒ a quarter.
    assert_eq!(
        incremental_mean_embedding(&[0.0], &[1.0], 3),
        vec![0.25_f32]
    );
}

#[test]
fn the_fallback_summary_names_the_turn_count_and_both_bookends() {
    let summary = fallback_summary("how do I start", "thanks, that worked", 7);
    assert!(summary.contains("7 turns"));
    assert!(summary.contains("how do I start"));
    assert!(summary.contains("thanks, that worked"));
}

#[test]
fn the_fallback_summary_truncates_on_a_char_boundary() {
    // 300 multi-byte chars: a byte-indexed truncation would panic here.
    let long: String = "é".repeat(300);
    let summary = fallback_summary(&long, "end", 2);
    assert!(summary.contains("..."));
    // 200 chars kept, not 200 bytes.
    assert_eq!(summary.matches('é').count(), 200);
}
