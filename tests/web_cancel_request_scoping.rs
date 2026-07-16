//! Request-scoped web-chat cancellation (#4760).
//!
//! Regression for a real in-flight-teardown bug: a client that timed out on
//! request A and then sent request B — which supersedes A on the same thread —
//! had A's late-arriving cancel tear down B, killing the newer turn at t=0
//! ("Cancelled by newer request"). The root cause was that `cancel_chat`
//! cancelled *whatever* turn was in flight for the thread, unscoped by
//! `request_id`.
//!
//! The fix routes the cancel decision through `cancel_should_target`: a scoped
//! cancel (`Some(request_id)`) only fires when it matches the turn actually in
//! flight, so a stale cancel for a superseded request becomes a no-op and the
//! newer turn survives. An unscoped cancel (`None` — Stop button / session
//! teardown) still stops whatever is running.
//!
//! These assertions pin that decision. `cancel_should_target` is a pure
//! predicate, so this is a fast, deterministic unit test with no runtime setup.

use openhuman_core::openhuman::web_chat::cancel_should_target;

#[test]
fn unscoped_cancel_always_targets_the_in_flight_turn() {
    // No request_id => "stop whatever is running on this thread" (the Stop
    // button / session-teardown path). It always targets the in-flight turn.
    assert!(cancel_should_target(None, "req-A"));
    assert!(cancel_should_target(None, "req-B"));
}

#[test]
fn scoped_cancel_fires_for_its_own_request() {
    // A client cancelling exactly the turn it started tears that turn down.
    assert!(cancel_should_target(Some("req-A"), "req-A"));
}

#[test]
fn stale_scoped_cancel_does_not_kill_a_newer_turn() {
    // The #4760 bug: request A timed out client-side and B is now the in-flight
    // turn on this thread. A's late, request-scoped cancel must NOT tear down B.
    assert!(!cancel_should_target(Some("req-A"), "req-B"));
    // Symmetric: a cancel naming an already-finished request is inert.
    assert!(!cancel_should_target(
        Some("old-request"),
        "current-request"
    ));
}
