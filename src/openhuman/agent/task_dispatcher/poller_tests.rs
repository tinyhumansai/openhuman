use super::*;

#[test]
fn next_poll_delay_holds_base_cadence_within_the_grace_window() {
    let base = Duration::from_secs(POLLER_TICK_SECONDS);
    // Fresh work and the first few idle ticks all poll at the base cadence.
    assert_eq!(next_poll_delay(0), base);
    assert_eq!(next_poll_delay(POLLER_IDLE_GRACE_TICKS), base);
}

#[test]
fn next_poll_delay_backs_off_exponentially_past_the_grace_window() {
    // One tick past the grace window doubles, then doubles again.
    assert_eq!(
        next_poll_delay(POLLER_IDLE_GRACE_TICKS + 1),
        Duration::from_secs(POLLER_TICK_SECONDS * 2)
    );
    assert_eq!(
        next_poll_delay(POLLER_IDLE_GRACE_TICKS + 2),
        Duration::from_secs(POLLER_TICK_SECONDS * 4)
    );
    assert_eq!(
        next_poll_delay(POLLER_IDLE_GRACE_TICKS + 3),
        Duration::from_secs(POLLER_TICK_SECONDS * 8)
    );
}

#[test]
fn next_poll_delay_saturates_at_the_ceiling() {
    let cap = Duration::from_secs(POLLER_MAX_BACKOFF_SECONDS);
    // A long idle streak caps out (self-suspend) and never overflows.
    assert_eq!(next_poll_delay(50), cap);
    assert_eq!(next_poll_delay(u32::MAX), cap);
    // The backoff is monotonic non-decreasing and never exceeds the ceiling.
    let mut prev = next_poll_delay(0);
    for idle in 1..40u32 {
        let d = next_poll_delay(idle);
        assert!(d >= prev, "backoff must not shrink as idle grows");
        assert!(d <= cap, "backoff must never exceed the ceiling");
        prev = d;
    }
}
