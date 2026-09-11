//! Tests for the turn-scoped sub-agent dispatch guard (issue #5804).
//!
//! The policy ([`decide`]) is exercised as a pure function so every branch is
//! reachable without a runtime, a clock, or a model. The scope helpers are
//! exercised under `#[tokio::test]` because a `task_local` needs a task.

use std::time::Duration;

use super::{
    check, current, decide, record_subagent_elapsed, with_dispatch_guard, DispatchDecision,
    DispatchInputs,
};

/// Baseline inputs: nothing known, nothing to refuse.
fn inputs() -> DispatchInputs {
    DispatchInputs {
        pause_requested: false,
        pause_completed_calls: 0,
        pause_cap: 0,
        remaining: None,
        observed_max: None,
        observed_samples: 0,
    }
}

#[test]
fn allows_when_nothing_is_known() {
    assert_eq!(decide(inputs()), DispatchDecision::Allow);
}

#[test]
fn refuses_once_a_graceful_pause_has_been_requested() {
    let decision = decide(DispatchInputs {
        pause_requested: true,
        pause_completed_calls: 15,
        pause_cap: 15,
        ..inputs()
    });
    assert_eq!(
        decision,
        DispatchDecision::RefusePaused {
            completed_model_calls: 15,
            cap: 15,
        },
        "a dispatch issued after the cap-pause request is the #5804 failure"
    );
}

#[test]
fn pause_refusal_does_not_depend_on_the_remaining_budget() {
    // The whole point: at the moment of the observed failure there was budget
    // left (26s), it just was not enough. The pause arm must fire on intent
    // alone, with an hour to spare.
    let decision = decide(DispatchInputs {
        pause_requested: true,
        pause_completed_calls: 3,
        pause_cap: 3,
        remaining: Some(Duration::from_secs(3600)),
        observed_max: Some(Duration::from_secs(1)),
        observed_samples: 9,
    });
    assert!(
        matches!(decision, DispatchDecision::RefusePaused { .. }),
        "pause is a fact about intent, not an estimate — budget must not overrule it"
    );
}

#[test]
fn refuses_when_less_remains_than_the_slowest_observed_subagent() {
    // The observed run: ~26s left, children averaging ~68s.
    let decision = decide(DispatchInputs {
        remaining: Some(Duration::from_millis(26_375)),
        observed_max: Some(Duration::from_secs(68)),
        observed_samples: 17,
        ..inputs()
    });
    assert_eq!(
        decision,
        DispatchDecision::RefuseBudget {
            remaining_ms: 26_375,
            observed_max_ms: 68_000,
            observed_samples: 17,
        },
        "a dispatch that cannot fit in the remaining budget must not be attempted"
    );
}

#[test]
fn allows_when_the_remaining_budget_still_covers_the_slowest_observed_subagent() {
    let decision = decide(DispatchInputs {
        remaining: Some(Duration::from_secs(300)),
        observed_max: Some(Duration::from_secs(68)),
        observed_samples: 17,
        ..inputs()
    });
    assert_eq!(decision, DispatchDecision::Allow);
}

#[test]
fn allows_on_the_exact_boundary() {
    // `remaining == observed_max` is not evidence the dispatch cannot finish,
    // so the guard must not refuse it. Only a strict shortfall refuses.
    let decision = decide(DispatchInputs {
        remaining: Some(Duration::from_secs(68)),
        observed_max: Some(Duration::from_secs(68)),
        observed_samples: 2,
        ..inputs()
    });
    assert_eq!(decision, DispatchDecision::Allow);
}

#[test]
fn allows_the_first_dispatch_however_little_budget_remains() {
    // No sub-agent has completed, so there is no sample to judge against and
    // the guard has no evidence. Refusing here would be a guess, and would
    // break every short turn that delegates exactly once.
    let decision = decide(DispatchInputs {
        remaining: Some(Duration::from_millis(1)),
        observed_max: None,
        observed_samples: 0,
        ..inputs()
    });
    assert_eq!(decision, DispatchDecision::Allow);
}

#[test]
fn allows_when_the_wall_clock_ceiling_is_disabled() {
    // `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS=0` → no ceiling → nothing can run out.
    let decision = decide(DispatchInputs {
        remaining: None,
        observed_max: Some(Duration::from_secs(600)),
        observed_samples: 40,
        ..inputs()
    });
    assert_eq!(decision, DispatchDecision::Allow);
}

#[test]
fn the_budget_rule_is_scale_free() {
    // Same rule, three orders of magnitude apart and at both fan-out extremes:
    // the decision depends only on remaining-vs-observed, never on how many
    // children have run or how long they took in absolute terms.
    let fast_small = decide(DispatchInputs {
        remaining: Some(Duration::from_millis(40)),
        observed_max: Some(Duration::from_millis(50)),
        observed_samples: 3,
        ..inputs()
    });
    let slow_large = decide(DispatchInputs {
        remaining: Some(Duration::from_secs(40)),
        observed_max: Some(Duration::from_secs(50)),
        observed_samples: 300,
        ..inputs()
    });
    assert!(matches!(fast_small, DispatchDecision::RefuseBudget { .. }));
    assert!(matches!(slow_large, DispatchDecision::RefuseBudget { .. }));
}

#[tokio::test]
async fn no_guard_outside_a_turn_scope() {
    assert!(current().is_none());
    assert_eq!(
        check(),
        DispatchDecision::Allow,
        "an absent guard must restore the previous behaviour exactly"
    );
    // Must not panic when there is nothing to record into.
    record_subagent_elapsed(Duration::from_secs(1));
}

#[tokio::test]
async fn pause_recorded_through_the_shared_handle_is_visible_to_the_dispatch_check() {
    // This is the wiring the fix depends on: the cap-pause listener holds a
    // clone of the same `Arc` and writes through it, while the sub-agent
    // runner reads the task-local. Both must see one state.
    with_dispatch_guard(Some(Duration::from_secs(600)), async {
        assert_eq!(check(), DispatchDecision::Allow);

        let state = current().expect("guard installed");
        state.record_pause_requested(15, 15);

        assert_eq!(
            check(),
            DispatchDecision::RefusePaused {
                completed_model_calls: 15,
                cap: 15,
            },
            "the runner must observe a pause the listener recorded"
        );
    })
    .await;
}

#[tokio::test]
async fn observed_durations_accumulate_as_a_running_maximum() {
    with_dispatch_guard(Some(Duration::from_secs(600)), async {
        record_subagent_elapsed(Duration::from_secs(5));
        record_subagent_elapsed(Duration::from_secs(90));
        record_subagent_elapsed(Duration::from_secs(30));

        // Read the real recorded state, not a hand-built copy of it: the claim
        // under test is that `record_subagent_elapsed` keeps the maximum
        // rather than the mean or the most recent value.
        let snapshot = current().expect("guard installed").snapshot();
        assert_eq!(
            snapshot.observed_max,
            Some(Duration::from_secs(90)),
            "the running maximum must survive a later, shorter sub-agent"
        );
        assert_eq!(snapshot.observed_samples, 3);
    })
    .await;
}

#[tokio::test]
async fn the_remaining_budget_is_measured_from_the_scope_and_shrinks() {
    with_dispatch_guard(Some(Duration::from_millis(400)), async {
        let state = current().expect("guard installed");
        let before = state.snapshot().remaining.expect("a ceiling is configured");
        tokio::time::sleep(Duration::from_millis(60)).await;
        let after = state.snapshot().remaining.expect("a ceiling is configured");
        assert!(
            after < before,
            "remaining budget must shrink with elapsed wall-clock ({after:?} !< {before:?})"
        );

        // With a sample longer than the whole ceiling, the guard now has
        // positive evidence and must refuse.
        record_subagent_elapsed(Duration::from_secs(60));
        assert!(
            matches!(check(), DispatchDecision::RefuseBudget { .. }),
            "a sub-agent longer than the entire remaining budget must block dispatch"
        );
    })
    .await;
}

#[tokio::test]
async fn the_guard_does_not_leak_into_a_detached_task() {
    // Background sub-agents run on tasks that do not inherit the task-local,
    // deliberately — same rule as the usage collector. They must degrade to
    // "allow", not panic.
    with_dispatch_guard(Some(Duration::from_secs(600)), async {
        current()
            .expect("guard installed")
            .record_pause_requested(1, 1);
        let detached = tokio::spawn(async { check() });
        assert_eq!(
            detached.await.expect("task joined"),
            DispatchDecision::Allow,
            "a detached task sees no guard and must behave as it did before"
        );
    })
    .await;
}

#[tokio::test]
async fn concurrent_same_task_dispatches_share_one_guard() {
    // `spawn_parallel_agents` fans out through `map_reduce`, which bounds
    // concurrency with `futures`' `buffer_unordered` rather than
    // `tokio::spawn` — so every parallel worker is polled on the caller's task
    // and inherits this task-local. `tokio::join!` has the same property, and
    // this pins the consequence: concurrency does not bypass the gate, and the
    // sample one worker records is visible to the next worker's check.
    with_dispatch_guard(Some(Duration::from_millis(50)), async {
        // Order matters, and getting it wrong here is what an earlier revision
        // of this test did: it recorded the 60s sample *before* its own
        // `check()`, so the first worker judged itself against a duration it
        // had just written and the `Allow` assertion could never hold. A worker
        // checks on the way in and records on the way out — mirror that.
        let a = async {
            let decision = check();
            record_subagent_elapsed(Duration::from_secs(60));
            decision
        };
        let b = async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            check()
        };
        let (first, second) = tokio::join!(a, b);

        // The first worker had no sample of its own to judge against yet.
        assert_eq!(first, DispatchDecision::Allow);
        // The second sees the first's sample through the shared guard, and the
        // budget is gone — exactly the refusal a serial run would produce.
        assert!(
            matches!(second, DispatchDecision::RefuseBudget { .. }),
            "a concurrently-polled dispatch must see the shared guard, not a fresh one: {second:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_parallel_batch_with_no_samples_is_never_refused() {
    // The other direction, and the one that matters for throughput: the FIRST
    // parallel batch has no completed sub-agent to learn from, so however many
    // workers it fans out to and however little budget is left, none of them
    // may be refused. A guard that blocked the opening fan-out would break
    // every parallel task rather than fixing one.
    with_dispatch_guard(Some(Duration::from_millis(1)), async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let worker = || async { check() };
        let decisions = tokio::join!(worker(), worker(), worker(), worker(), worker());
        let all = [
            decisions.0,
            decisions.1,
            decisions.2,
            decisions.3,
            decisions.4,
        ];
        assert!(
            all.iter().all(|d| *d == DispatchDecision::Allow),
            "an opening fan-out has no evidence against it and must proceed: {all:?}"
        );
    })
    .await;
}
