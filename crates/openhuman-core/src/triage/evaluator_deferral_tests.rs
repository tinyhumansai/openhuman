use super::super::chain::record_outage;
use super::*;
use tinyagents_harness::retry::{PersistentFailoverBreaker, PersistentFailoverBreakerConfig};

fn wall_clock_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[test]
fn outage_backoff_is_exponential_and_has_terminal_limit() {
    let state = new_retry_state();
    let delays = [30_000, 60_000, 120_000, 240_000, 480_000, 900_000, 900_000];
    let mut now = 0;

    for delay in delays {
        let mut attempt = state
            .try_acquire("stub-cloud".to_string(), now)
            .expect("attempt");
        let failure = attempt.record_failure(now).expect("failure");
        assert!(!failure.failure_limit_reached);
        assert_eq!(failure.next_attempt_ms.saturating_sub(now), delay);
        now = failure.next_attempt_ms;
    }

    let mut attempt = state
        .try_acquire("stub-cloud".to_string(), now)
        .expect("terminal attempt");
    let failure = attempt.record_failure(now).expect("terminal failure");
    assert!(failure.failure_limit_reached);
    assert_eq!(failure.failure_count, 8);
    assert!(begin_outage_attempt(Some(&state), "stub-cloud").is_none());
}

#[test]
fn outage_failure_maps_to_deferred_then_terminal_host_outcomes() {
    let state = PersistentFailoverBreaker::new(PersistentFailoverBreakerConfig {
        base_backoff_ms: 0,
        max_backoff_ms: 0,
        failure_limit: 8,
    });

    for failure in 1..=8 {
        let mut attempt = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");
        let before = wall_clock_ms();
        let outcome = record_outage(Some(&mut attempt)).expect("host outcome");
        let after = wall_clock_ms();
        if failure < 8 {
            match outcome {
                TriageOutcome::Deferred {
                    defer_until_ms,
                    reason,
                } => {
                    assert_eq!(reason, "managed backend outage; retry 0ms backoff");
                    assert!((before..=after).contains(&defer_until_ms));
                }
                TriageOutcome::Decision(_) | TriageOutcome::Terminal { .. } => {
                    panic!("failure {failure} should remain deferred")
                }
            }
        } else {
            match outcome {
                TriageOutcome::Terminal { reason } => {
                    assert!(reason.contains("retry limit (8)"));
                }
                TriageOutcome::Decision(_) | TriageOutcome::Deferred { .. } => {
                    panic!("failure 8 should reach the terminal limit")
                }
            }
        }
    }
}

#[test]
fn outage_state_isolated_by_model() {
    let state = new_retry_state();
    let mut first = state
        .try_acquire("managed:model-a".to_string(), 0)
        .expect("first model attempt");
    first.record_failure(0).expect("first outage");
    assert!(state.try_acquire("managed:model-b".to_string(), 0).is_ok());
    assert!(state.try_acquire("managed:model-a".to_string(), 1).is_err());
}

#[tokio::test]
async fn cloud_safety_flagged_without_local_returns_terminal_not_err() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("Prompt flagged for security review and was not processed.".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("safety-flagged with no local must be terminal, not Err");

    match outcome {
        TriageOutcome::Terminal { reason } => {
            assert!(
                reason.to_lowercase().contains("prompt-guard"),
                "deferral reason should name the prompt-guard cause: {reason}"
            );
        }
        TriageOutcome::Decision(_) | TriageOutcome::Deferred { .. } => {
            panic!("expected Terminal")
        }
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "no retry — guard would block the second cloud call too"
    );
}

#[tokio::test]
async fn no_local_arm_returns_terminal_after_cloud_exhaustion() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("HTTP 503 Service Unavailable".to_string())
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("Terminal is Ok");

    match outcome {
        TriageOutcome::Terminal { reason } => {
            assert!(
                reason.contains("local arm unavailable"),
                "reason should explain the missing local arm: {reason}"
            );
        }
        TriageOutcome::Decision(_) | TriageOutcome::Deferred { .. } => {
            panic!("expected Terminal")
        }
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud + 1 retry, no local"
    );
}

#[tokio::test]
async fn repeated_no_local_failures_back_off_before_calling_cloud_again() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);
    let state = new_retry_state();

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("HTTP 503 Service Unavailable".to_string())
        }
    })
    .await;

    let first = run_triage_with_arms_for_test_with_state(cloud_arm(), None, &envelope(), &state)
        .await
        .expect("first outage result");
    assert!(matches!(first, TriageOutcome::Deferred { .. }));
    assert_eq!(counter.load(Ordering::SeqCst), 2, "initial call plus retry");

    let second = run_triage_with_arms_for_test_with_state(cloud_arm(), None, &envelope(), &state)
        .await
        .expect("backoff result");
    assert!(matches!(second, TriageOutcome::Deferred { .. }));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "an open backoff window must not invoke the cloud again"
    );
}

#[tokio::test]
async fn double_cloud_parse_failure_falls_through_to_local_fallback() {
    // Regression for #2322: two malformed cloud replies used to turn the
    // second cloud parse error into ArmError::Fatal, bubbling out of
    // run_triage as Err and making the Composio subscriber emit
    // `[composio][triage] run_triage failed` at error level.
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                assert_eq!(
                    req.provider_name, "stub-cloud",
                    "first two attempts should stay on the cloud arm"
                );
                Ok(AgentTurnResponse::new("not json"))
            } else {
                assert_eq!(
                    req.provider_name, "stub-local",
                    "malformed cloud retry should fall through to local"
                );
                Ok(AgentTurnResponse::new(VALID_JSON_REPLY))
            }
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), Some(local_arm()), &envelope())
        .await
        .expect("malformed cloud retry must fall through, not surface Err");

    let run = outcome.into_decision().expect("decision");
    assert_eq!(run.resolution_path, TriageResolutionPath::LocalFallback);
    assert!(run.used_local);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "1 cloud + 1 cloud retry + 1 local"
    );
}

#[tokio::test]
async fn double_cloud_parse_failure_without_local_returns_terminal_not_err() {
    AgentDefinitionRegistry::init_global_builtins().expect("init_global_builtins");
    let counter = StdArc::new(AtomicUsize::new(0));
    let counter_for_stub = StdArc::clone(&counter);

    let _guard = mock_agent_run_turn(move |_req| {
        let counter = StdArc::clone(&counter_for_stub);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(AgentTurnResponse::new("still not json"))
        }
    })
    .await;

    let outcome = run_triage_with_arms_for_test(cloud_arm(), None, &envelope())
        .await
        .expect("malformed cloud retry with no local must be terminal, not Err");

    match outcome {
        TriageOutcome::Terminal { reason } => {
            assert!(
                reason.contains("local arm unavailable"),
                "reason should explain the missing local arm: {reason}"
            );
        }
        TriageOutcome::Decision(_) | TriageOutcome::Deferred { .. } => {
            panic!("expected Terminal")
        }
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud + 1 cloud retry, no local"
    );
}

#[tokio::test]
async fn in_flight_breaker_defers_before_dispatch_for_one_second() {
    let state = new_retry_state();
    let attempt =
        begin_outage_attempt(Some(&state), "stub-cloud:stub-cloud-model").expect("held attempt");
    let before = wall_clock_ms();

    let outcome = run_triage_with_arms_for_test_with_state(cloud_arm(), None, &envelope(), &state)
        .await
        .expect("deferred outcome");
    let after = wall_clock_ms();

    match outcome {
        TriageOutcome::Deferred { defer_until_ms, .. } => {
            assert!((before + 1_000..=after + 1_000).contains(&defer_until_ms));
        }
        TriageOutcome::Decision(_) | TriageOutcome::Terminal { .. } => {
            panic!("expected an in-flight deferral")
        }
    }
    drop(attempt);
}
