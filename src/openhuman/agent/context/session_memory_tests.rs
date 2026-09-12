use super::*;

#[test]
fn default_state_does_not_extract() {
    let state = SessionMemoryState::default();
    let cfg = SessionMemoryConfig::default();
    assert!(!state.should_extract(&cfg));
}

#[test]
fn all_three_thresholds_must_be_crossed() {
    let cfg = SessionMemoryConfig::default();

    // Only token threshold crossed → no.
    let mut s = SessionMemoryState::default();
    s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
    assert!(!s.should_extract(&cfg));

    // Tokens + tool calls, no turn growth → no.
    s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
    assert!(!s.should_extract(&cfg));

    // All three crossed → yes.
    s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
    assert!(s.should_extract(&cfg));
}

#[test]
fn in_progress_suppresses_extraction() {
    let cfg = SessionMemoryConfig::default();
    let mut s = SessionMemoryState::default();
    s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
    s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
    s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
    assert!(s.should_extract(&cfg));
    s.mark_extraction_started();
    assert!(!s.should_extract(&cfg));
}

#[test]
fn mark_complete_resets_deltas() {
    let cfg = SessionMemoryConfig::default();
    let mut s = SessionMemoryState::default();
    s.total_tokens = 10_000;
    s.total_tool_calls = 15;
    s.current_turn = 10;
    s.mark_extraction_started();
    s.mark_extraction_complete();

    // Immediately after completion no further extraction should
    // fire until the deltas are re-crossed.
    assert!(!s.should_extract(&cfg));

    // Grow each counter past threshold again.
    s.total_tokens += DEFAULT_MIN_TOKEN_GROWTH;
    s.total_tool_calls += DEFAULT_MIN_TOOL_CALLS;
    s.current_turn += DEFAULT_MIN_TURNS_BETWEEN;
    assert!(s.should_extract(&cfg));
}

#[test]
fn mark_failed_leaves_deltas_intact() {
    let cfg = SessionMemoryConfig::default();
    let mut s = SessionMemoryState::default();
    s.total_tokens = DEFAULT_MIN_TOKEN_GROWTH + 1;
    s.total_tool_calls = DEFAULT_MIN_TOOL_CALLS + 1;
    s.current_turn = DEFAULT_MIN_TURNS_BETWEEN + 1;
    s.mark_extraction_started();
    s.mark_extraction_failed();

    // Should still fire on the next attempt because the
    // "last_extract" counters were not advanced.
    assert!(s.should_extract(&cfg));
}

#[test]
fn record_usage_is_monotonic() {
    let mut s = SessionMemoryState::default();
    s.record_usage(5_000);
    s.record_usage(3_000); // regression — must not decrease.
    assert_eq!(s.total_tokens, 5_000);
    s.record_usage(7_500);
    assert_eq!(s.total_tokens, 7_500);
}

#[test]
fn tick_turn_increments() {
    let mut s = SessionMemoryState::default();
    s.tick_turn();
    s.tick_turn();
    s.tick_turn();
    assert_eq!(s.current_turn, 3);
}

#[test]
fn record_tool_calls_accumulates() {
    let mut s = SessionMemoryState::default();
    s.record_tool_calls(3);
    s.record_tool_calls(2);
    assert_eq!(s.total_tool_calls, 5);
}
