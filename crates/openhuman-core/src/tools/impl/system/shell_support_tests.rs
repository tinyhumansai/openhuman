use super::*;

const LONG: Duration = Duration::from_secs(3600);

#[test]
fn a_call_comes_back_after_two_minutes_by_default() {
    assert_eq!(
        yield_after(None, LONG, "make -j8"),
        Some(Duration::from_secs(DEFAULT_YIELD_AFTER_SECS))
    );
}

#[test]
fn a_requested_time_is_kept_up_to_ten_minutes() {
    assert_eq!(
        yield_after(Some(300), Duration::from_secs(7200), "make"),
        Some(Duration::from_secs(300))
    );
    assert_eq!(
        yield_after(Some(3600), Duration::from_secs(86_400), "make"),
        Some(Duration::from_secs(MAX_YIELD_AFTER_SECS))
    );
}

#[test]
fn zero_waits_for_the_end() {
    assert_eq!(yield_after(Some(0), LONG, "make"), None);
}

#[test]
fn a_deliberate_wait_is_not_cut_short() {
    assert_eq!(yield_after(None, LONG, "sleep 100; tail run.log"), None);
    assert_eq!(yield_after(Some(30), LONG, "  sleep 5"), None);
    assert_eq!(
        yield_after(None, LONG, "sleep 240 && tail -3 /app/run.log; ls -la /app"),
        None
    );
    assert!(yield_after(None, LONG, "sleeper --run").is_some());
    assert!(yield_after(None, LONG, "make; sleep 5").is_some());
    assert!(
        yield_after(None, LONG, "sleep 2; make -j8").is_some(),
        "real work after the sleep still yields"
    );
}

#[test]
fn the_yield_point_stays_before_a_short_deadline() {
    assert_eq!(
        yield_after(None, Duration::from_secs(20), "make"),
        Some(Duration::from_secs(10))
    );
    assert_eq!(yield_after(None, Duration::from_secs(1), "make"), None);
}

#[test]
fn a_short_deadline_brings_the_yield_point_forward() {
    // An eighth of the deadline: 800 s left to run comes back at 100 s.
    assert_eq!(
        yield_after(None, Duration::from_secs(800), "make"),
        Some(Duration::from_secs(100))
    );
    // Never earlier than 30 s for that cap.
    assert_eq!(
        yield_after(None, Duration::from_secs(120), "make"),
        Some(Duration::from_secs(MIN_YIELD_AFTER_SECS))
    );
    // A shorter request still wins.
    assert_eq!(
        yield_after(Some(5), Duration::from_secs(120), "make"),
        Some(Duration::from_secs(5))
    );
}
