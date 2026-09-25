use super::*;

#[test]
fn outage_backoff_is_exponential_and_has_terminal_limit() {
    let state = std::sync::Mutex::new(std::collections::HashMap::new());
    let delays = [30_000, 60_000, 120_000, 240_000, 480_000, 900_000, 900_000];

    for delay in delays {
        let generation = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");
        let outcome =
            record_outage(Some(&state), "stub-cloud", Some(generation)).expect("backoff outcome");
        match outcome {
            TriageOutcome::Deferred { reason, .. } => {
                assert!(reason.contains(&format!("{delay}ms backoff")));
            }
            TriageOutcome::Decision(_) | TriageOutcome::Terminal { .. } => {
                panic!("outage should remain retryable before the limit")
            }
        }
        state
            .lock()
            .expect("test state lock")
            .get_mut("stub-cloud")
            .expect("recorded outage")
            .next_attempt_ms = 0;
    }

    let generation = begin_outage_attempt(Some(&state), "stub-cloud").expect("attempt");
    let outcome =
        record_outage(Some(&state), "stub-cloud", Some(generation)).expect("terminal outcome");
    assert!(matches!(outcome, TriageOutcome::Terminal { .. }));
}

#[test]
fn outage_state_isolated_by_model() {
    let state = std::sync::Mutex::new(std::collections::HashMap::new());
    let generation = begin_outage_attempt(Some(&state), "managed:model-a").expect("attempt");
    let first =
        record_outage(Some(&state), "managed:model-a", Some(generation)).expect("first outage");
    assert!(matches!(first, TriageOutcome::Deferred { .. }));
    let generation = begin_outage_attempt(Some(&state), "managed:model-b").expect("attempt");
    assert!(record_outage(Some(&state), "managed:model-b", Some(generation)).is_some());
    let states = state.lock().expect("test state lock");
    assert_eq!(states.len(), 2, "models must not share outage windows");
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
    let state = std::sync::Mutex::new(std::collections::HashMap::new());

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
