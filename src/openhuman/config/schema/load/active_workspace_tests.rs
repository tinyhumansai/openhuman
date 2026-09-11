//! The active-workspace state machine.
//!
//! These drive a local [`ActiveWorkspace`] rather than the process-global
//! slot, and that is load-bearing rather than stylistic. `Config::load_or_init`
//! publishes into the global, thousands of tests in this binary load a config,
//! and they run in parallel — a test that pinned the global would pass on its
//! own and fail whenever it happened to interleave with one of them. Driving
//! an owned instance makes each case deterministic and independent.

use std::path::PathBuf;

use super::ActiveWorkspace;

fn ws(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}tmp{}oh-{name}{}workspace",
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    ))
}

#[test]
fn starts_unknown_rather_than_guessing() {
    let state = ActiveWorkspace::default();
    assert_eq!(state.current, None);
    assert_eq!(state.announced, None);
}

#[test]
fn a_first_publish_is_readable_and_announces() {
    let mut state = ActiveWorkspace::default();
    assert!(
        state.publish(&ws("a")).transition.is_some(),
        "the first resolve is news"
    );
    assert_eq!(state.current, Some(ws("a")));
}

#[test]
fn a_later_publish_replaces_the_previous_workspace_and_announces() {
    let mut state = ActiveWorkspace::default();
    state.publish(&ws("a"));
    assert!(
        state.publish(&ws("b")).transition.is_some(),
        "a different workspace is a switch"
    );
    assert_eq!(state.current, Some(ws("b")));
}

/// The loader runs on far more than switches — every `Config::load_or_init` —
/// so announcing each one would fill the Event Log with phantom workspace
/// changes and make a real one impossible to spot.
#[test]
fn republishing_the_same_workspace_announces_once() {
    let mut state = ActiveWorkspace::default();
    assert!(state.publish(&ws("a")).transition.is_some());
    assert!(state.publish(&ws("a")).transition.is_none());
    assert!(state.publish(&ws("a")).transition.is_none());
    assert_eq!(state.current, Some(ws("a")));
}

/// A marker write says the answer changed, not what it changed *to*, so the
/// readable value goes back to "unknown" and the next resolve refills it.
/// Leaving the previous value in place is the bug this guards: a
/// switched-away workspace would keep reading as active.
#[test]
fn invalidation_clears_the_readable_value() {
    let mut state = ActiveWorkspace::default();
    state.publish(&ws("a"));
    assert!(state.invalidate(), "there was something to clear");
    assert_eq!(state.current, None);
}

#[test]
fn invalidating_an_already_empty_slot_is_harmless() {
    let mut state = ActiveWorkspace::default();
    assert!(!state.invalidate());
    assert!(!state.invalidate());
    assert_eq!(state.current, None);
}

/// Invalidation says "the answer may have changed", not "the answer changed".
/// Signing in as the user who is already active rewrites `active_user.toml`
/// with the same id, so the resolve that follows lands on the workspace that
/// was already current. Announcing that would put a switch in the Event Log
/// that never happened.
#[test]
fn re_resolving_the_same_workspace_after_invalidation_does_not_re_announce() {
    let mut state = ActiveWorkspace::default();
    state.publish(&ws("a"));
    state.invalidate();

    assert!(
        state.publish(&ws("a")).transition.is_none(),
        "the workspace never actually changed"
    );
    assert_eq!(state.current, Some(ws("a")));
}

/// The other half of that rule: a marker write that *does* change the
/// workspace must still announce, or a consumer holding a long-lived stream
/// never learns it switched.
#[test]
fn a_different_workspace_after_invalidation_is_announced() {
    let mut state = ActiveWorkspace::default();
    state.publish(&ws("a"));
    state.invalidate();

    assert!(state.publish(&ws("b")).transition.is_some());
    assert_eq!(state.current, Some(ws("b")));
}

/// Invalidation must not reset the announce memory, or the pair
/// invalidate-then-republish would announce a phantom switch every time.
#[test]
fn invalidation_leaves_the_announce_memory_intact() {
    let mut state = ActiveWorkspace::default();
    state.publish(&ws("a"));
    state.invalidate();
    assert_eq!(state.announced, Some(ws("a")));
}

/// The global wrapper reaches for the event bus, which no unit test stands
/// up. `BUS.publish` is documented as a no-op before `init`; this pins that
/// the cache does not depend on the bus being up, and that the wrapper does
/// not panic on the path every boot takes.
#[test]
fn the_global_publish_survives_an_uninitialised_bus() {
    super::publish_active_workspace(&ws("no-bus"));
    // No assertion on the readable value: other tests in this binary load
    // configs concurrently and publish into the same slot. The contract under
    // test is only that this path does not panic without a bus.
}

/// The interleaving CodeRabbit flagged. The bus publish happens outside the
/// lock, so two resolvers can commit in one order and reach the emit in the
/// other. Without a revision, the stale emit wins and — because `announced`
/// already holds the newer workspace — no later resolution ever corrects it,
/// leaving every client on the workspace the user left.
#[test]
fn a_superseded_transition_is_not_announced() {
    let mut state = ActiveWorkspace::default();

    let first = state
        .publish(&ws("a"))
        .transition
        .expect("first resolve announces");
    let second = state
        .publish(&ws("b"))
        .transition
        .expect("a switch announces");

    // B committed after A, so only B may reach the bus.
    assert!(!state.is_current(first.revision), "A's emit is superseded");
    assert!(
        state.is_current(second.revision),
        "B's emit is the live one"
    );
}

#[test]
fn revisions_increase_only_for_announced_transitions() {
    let mut state = ActiveWorkspace::default();

    let first = state.publish(&ws("a")).transition.expect("announces");
    assert!(
        state.publish(&ws("a")).transition.is_none(),
        "same workspace is silent"
    );
    let second = state.publish(&ws("b")).transition.expect("announces");

    assert!(
        second.revision > first.revision,
        "a real switch must outrank the one before it"
    );
    assert_eq!(
        state.revision, second.revision,
        "a suppressed republish must not consume a revision"
    );
}

/// Invalidation is not a transition — it says the answer is unknown, not that
/// it changed. Bumping the revision there would make every marker write look
/// newer than a switch that had already been announced.
#[test]
fn invalidation_does_not_consume_a_revision() {
    let mut state = ActiveWorkspace::default();
    let announced = state.publish(&ws("a")).transition.expect("announces");
    state.invalidate();
    assert_eq!(state.revision, announced.revision);
    assert!(state.is_current(announced.revision));
}

/// The revision a resolve reports must be the one *its* workspace is current
/// under — including when the resolve is not a transition. A consumer sending
/// the pair to a client takes both from this one call; if a silent republish
/// reported a stale or zero revision, the connect-time seed would rank below
/// every switch and be discarded.
#[test]
fn a_resolve_reports_the_revision_of_the_workspace_it_resolved() {
    let mut state = ActiveWorkspace::default();

    let first = state.publish(&ws("a"));
    let again = state.publish(&ws("a"));
    assert!(again.transition.is_none(), "not a transition");
    assert_eq!(
        again.revision, first.revision,
        "a silent republish reports the revision it was announced under"
    );

    let second = state.publish(&ws("b"));
    assert!(second.revision > first.revision);

    state.invalidate();
    let after = state.publish(&ws("b"));
    assert!(after.transition.is_none());
    assert_eq!(after.revision, second.revision);
}
