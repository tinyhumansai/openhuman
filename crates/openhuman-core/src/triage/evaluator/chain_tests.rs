use super::*;

#[test]
fn dropped_attempt_releases_in_flight_state() {
    let state = new_retry_state();
    let attempt = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");
    drop(attempt);
    assert!(begin_outage_attempt(Some(&state), "stub-cloud").is_some());
}

#[test]
fn monotonic_clock_does_not_move_backwards() {
    let before = monotonic_ms();
    let after = monotonic_ms();
    assert!(after >= before);
}

#[test]
fn only_one_initial_outage_attempt_can_be_in_flight() {
    let state = new_retry_state();

    let first = begin_outage_attempt(Some(&state), "stub-cloud").expect("first attempt");
    assert!(begin_outage_attempt(Some(&state), "stub-cloud").is_none());
    drop(first);
}

#[test]
fn unguarded_success_does_not_clear_another_in_flight_attempt() {
    let state = new_retry_state();
    let mut attempt = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");

    clear_outage(None);

    assert!(begin_outage_attempt(Some(&state), "stub-cloud").is_none());
    attempt.record_success();
}

#[test]
fn stale_failure_after_concurrent_recovery_remains_deferred() {
    let state = new_retry_state();
    let mut attempt = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");

    clear_outage(Some(&mut attempt));

    assert!(matches!(
        record_outage(Some(&mut attempt)),
        Some(TriageOutcome::Deferred { .. })
    ));
}

#[test]
fn in_flight_rejection_preserves_one_second_requeue_delay() {
    assert_eq!(rejection_delay_ms(BreakerRejection::InFlight, 123), 1_000);
}

#[test]
fn backoff_rejection_preserves_host_one_second_floor() {
    assert_eq!(
        rejection_delay_ms(
            BreakerRejection::Backoff {
                next_attempt_ms: 350,
                failure_count: 1,
                failure_limit_reached: false,
            },
            100,
        ),
        1_000
    );
    assert_eq!(
        rejection_delay_ms(
            BreakerRejection::Backoff {
                next_attempt_ms: 1_600,
                failure_count: 1,
                failure_limit_reached: false,
            },
            100,
        ),
        1_500
    );
}
