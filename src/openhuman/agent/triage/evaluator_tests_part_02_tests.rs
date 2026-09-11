use super::*;

#[tokio::test]
async fn cloud_safety_flagged_without_local_returns_deferred_not_err() {
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
        .expect("safety-flagged with no local must Defer, not Err");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.to_lowercase().contains("prompt-guard"),
                "deferral reason should name the prompt-guard cause: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "no retry — guard would block the second cloud call too"
    );
}

#[tokio::test]
async fn no_local_arm_returns_deferred_after_cloud_exhaustion() {
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
        .expect("Deferred is Ok");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.contains("local arm unavailable"),
                "reason should explain the missing local arm: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud + 1 retry, no local"
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
async fn double_cloud_parse_failure_without_local_returns_deferred_not_err() {
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
        .expect("malformed cloud retry with no local must Defer, not Err");

    match outcome {
        TriageOutcome::Deferred { reason, .. } => {
            assert!(
                reason.contains("local arm unavailable"),
                "reason should explain the missing local arm: {reason}"
            );
        }
        TriageOutcome::Decision(_) => panic!("expected Deferred"),
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "1 cloud + 1 cloud retry, no local"
    );
}
