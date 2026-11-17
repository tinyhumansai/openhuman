use super::*;

#[test]
fn cancellation_signals_the_matching_registered_turn() {
    let thread_id = "build-registry-test:thread-1";
    let token = CancellationToken::new();
    register_build_turn(
        thread_id.to_string(),
        Some("req-1".to_string()),
        token.clone(),
    );

    assert!(!token.is_cancelled());
    assert!(cancel_build_turn_scoped(thread_id, Some("req-1")));
    assert!(token.is_cancelled());

    unregister_build_turn(thread_id, Some("req-1"));
}

#[test]
fn cancellation_returns_false_when_no_turn_is_registered() {
    assert!(!cancel_build_turn_scoped(
        "build-registry-test:never-registered",
        Some("req-x")
    ));
    assert!(!cancel_build_turn_scoped(
        "build-registry-test:never-registered",
        None
    ));
}

#[test]
fn cancellation_ignores_a_mismatched_request_id() {
    let thread_id = "build-registry-test:thread-2";
    let token = CancellationToken::new();
    register_build_turn(
        thread_id.to_string(),
        Some("req-1".to_string()),
        token.clone(),
    );

    assert!(!cancel_build_turn_scoped(
        thread_id,
        Some("some-other-request")
    ));
    assert!(!token.is_cancelled());
    assert!(cancel_build_turn_scoped(thread_id, Some("req-1")));
    assert!(token.is_cancelled());

    unregister_build_turn(thread_id, Some("req-1"));
}

#[test]
fn unscoped_cancellation_targets_the_registered_turn() {
    let thread_id = "build-registry-test:thread-3";
    let token = CancellationToken::new();
    register_build_turn(
        thread_id.to_string(),
        Some("req-1".to_string()),
        token.clone(),
    );

    assert!(cancel_build_turn_scoped(thread_id, None));
    assert!(token.is_cancelled());
    unregister_build_turn(thread_id, None);
}

#[test]
fn stale_unregister_keeps_the_replacement_turn_registered() {
    let thread_id = "build-registry-test:thread-4";
    let token_a = CancellationToken::new();
    register_build_turn(
        thread_id.to_string(),
        Some("req-a".to_string()),
        token_a.clone(),
    );

    let token_b = CancellationToken::new();
    register_build_turn(
        thread_id.to_string(),
        Some("req-b".to_string()),
        token_b.clone(),
    );
    assert!(token_a.is_cancelled());
    assert!(!token_b.is_cancelled());

    unregister_build_turn(thread_id, Some("req-a"));
    assert!(cancel_build_turn_scoped(thread_id, Some("req-b")));
    assert!(token_b.is_cancelled());

    unregister_build_turn(thread_id, Some("req-b"));
    assert!(!cancel_build_turn_scoped(thread_id, None));
}

#[test]
fn unscoped_unregister_removes_the_current_turn() {
    let thread_id = "build-registry-test:thread-5";
    register_build_turn(
        thread_id.to_string(),
        Some("req-1".to_string()),
        CancellationToken::new(),
    );

    unregister_build_turn(thread_id, None);
    assert!(!cancel_build_turn_scoped(thread_id, None));
}
