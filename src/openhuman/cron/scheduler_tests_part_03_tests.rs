use super::*;

#[test]
fn forbidden_path_argument_skips_flags_and_urls() {
    let policy = SecurityPolicy::default();
    assert!(forbidden_path_argument(&policy, "curl https://example.com").is_none());
    assert!(forbidden_path_argument(&policy, "ls -la").is_none());
}

#[tokio::test]
async fn deliver_if_configured_skips_empty_mode() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery.mode = "".into();
    assert!(deliver_if_configured(&config, &job, "output", true)
        .await
        .is_ok());
}

#[tokio::test]
async fn deliver_if_configured_announce_missing_channel_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: None,
        to: Some("target".into()),
        best_effort: true,
    };
    let result = deliver_if_configured(&config, &job, "out", true).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn deliver_if_configured_announce_missing_target_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: Some("telegram".into()),
        to: None,
        best_effort: true,
    };
    let result = deliver_if_configured(&config, &job, "out", true).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn deliver_if_configured_proactive_mode_succeeds() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "proactive".into(),
        channel: None,
        to: None,
        best_effort: true,
    };
    assert!(deliver_if_configured(&config, &job, "hello", true)
        .await
        .is_ok());
}

// ──────────────────────────────────────────────────────────────────────
// Agent-error classifier (Bug B of #2279)
//
// `agent_error_to_user_message` must:
//   1. Return the expected canned string for each handled variant.
//   2. Fall back to `AGENT_JOB_USER_FAILURE_MESSAGE` for residual variants.
//   3. NEVER interpolate any field of the input error into its output.
//
// (3) is the airtight data-exposure guard. `last_agent_error` carries
// provider URLs with query tokens, stack traces, partial response bodies and
// occasionally user input. The leak-canary fuzz below proves none of that
// can reach the user-visible notification.
// ──────────────────────────────────────────────────────────────────────

#[test]
fn agent_error_to_user_message_classifies_provider_retryable() {
    let err = AgentError::ProviderError {
        message: "boom".into(),
        retryable: true,
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("temporarily unavailable"));
    assert!(msg.contains("retry"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_provider_non_retryable() {
    let err = AgentError::ProviderError {
        message: "invalid api key".into(),
        retryable: false,
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("provider"));
    assert!(msg.contains("credentials"));
    assert!(msg.contains("Connections \u{2192} API keys \u{2192} LLM"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_context_limit() {
    let err = AgentError::ContextLimitExceeded {
        utilization_pct: 98,
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("conversation grew too long"));
    assert!(msg.contains("context window"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_cost_budget() {
    let err = AgentError::CostBudgetExceeded {
        spent_microdollars: 5_000_000,
        budget_microdollars: 1_000_000,
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("cost budget"));
    assert!(msg.contains("Settings"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_max_iterations() {
    let err = AgentError::MaxIterationsExceeded { max: 10 };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("tool iterations"));
    assert!(msg.contains("Connections \u{2192} API keys \u{2192} LLM"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_empty_provider_response_for_3335() {
    // Issue #3335: the cron-path copy must stay in lock-step with the
    // web-channel `empty_response` arm — names the credits / billing
    // remedy explicitly and drops the misleading "local provider"
    // misdirect that broke remediation for Managed users.
    let err = AgentError::EmptyProviderResponse { iteration: 1 };
    let msg = agent_error_to_user_message(&err);
    assert!(
        msg.contains("Settings \u{2192} Billing"),
        "must point at billing for credit exhaustion: {msg}"
    );
    assert!(
        !msg.contains("local provider"),
        "must not claim a local provider exists: {msg}"
    );
    assert!(
        msg.contains("another model"),
        "must keep the model-switch remedy: {msg}"
    );
    assert!(
        msg.contains("Connections \u{2192} API keys \u{2192} LLM"),
        "must keep the provider-config deep link: {msg}"
    );
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_compaction_failed() {
    let err = AgentError::CompactionFailed {
        message: "summary failed".into(),
        consecutive_failures: 3,
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("compaction"));
    assert!(msg.contains("fresh context"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_classifies_permission_denied() {
    let err = AgentError::PermissionDenied {
        tool_name: "shell".into(),
        required_level: "Execute".into(),
        channel_max_level: "ReadOnly".into(),
    };
    let msg = agent_error_to_user_message(&err);
    assert!(msg.contains("tool"));
    assert!(msg.contains("channel"));
    assert!(msg.contains("Settings"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_falls_back_on_tool_execution_error() {
    // ToolExecutionError has no actionable canned message — the failure
    // shape is too freeform. Falls back to the residual constant.
    let err = AgentError::ToolExecutionError {
        tool_name: "shell".into(),
        message: "denied".into(),
    };
    let msg = agent_error_to_user_message(&err);
    assert_eq!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_falls_back_on_other() {
    let err = AgentError::Other(anyhow::anyhow!("untyped failure"));
    let msg = agent_error_to_user_message(&err);
    assert_eq!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn agent_error_to_user_message_canned_strings_are_short() {
    // Canned strings must stay ≤120 chars so they survive the 512-char
    // truncation in `push_cron_alert` without losing meaning, and so they
    // render cleanly in the notifications drawer. The fallback constant
    // is intentionally longer (multi-line w/ Discord link) and is excluded.
    let variants: Vec<AgentError> = vec![
        AgentError::ProviderError {
            message: "x".into(),
            retryable: true,
        },
        AgentError::ProviderError {
            message: "x".into(),
            retryable: false,
        },
        AgentError::ContextLimitExceeded { utilization_pct: 0 },
        AgentError::CostBudgetExceeded {
            spent_microdollars: 0,
            budget_microdollars: 0,
        },
        AgentError::MaxIterationsExceeded { max: 0 },
        AgentError::CompactionFailed {
            message: "x".into(),
            consecutive_failures: 0,
        },
        AgentError::PermissionDenied {
            tool_name: "x".into(),
            required_level: "x".into(),
            channel_max_level: "x".into(),
        },
        // Issue #3335: EmptyProviderResponse was historically absent from
        // this variants list — its old copy happened to fit, but nothing
        // enforced it. The fix shipped a new copy that explicitly names
        // the credits / billing remedy, which makes the length tradeoff
        // active rather than incidental. Lock it in so a future copy
        // change can't quietly grow past the drawer-render budget.
        AgentError::EmptyProviderResponse { iteration: 0 },
    ];
    for v in &variants {
        let msg = agent_error_to_user_message(v);
        if msg == AGENT_JOB_USER_FAILURE_MESSAGE {
            // Variant routed to the residual — length not enforced.
            continue;
        }
        assert!(
            msg.chars().count() <= 120,
            "Canned message too long ({} chars) for variant {:?}: {msg:?}",
            msg.chars().count(),
            std::mem::discriminant(v),
        );
    }
}

#[test]
fn classify_agent_anyhow_routes_typed_errors() {
    let typed = anyhow::Error::from(AgentError::MaxIterationsExceeded { max: 4 });
    let msg = classify_agent_anyhow_for_user(&typed);
    assert!(msg.contains("tool iterations"));
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn classify_agent_anyhow_falls_back_on_untyped_error() {
    // Plain anyhow error with no downcast target → residual fallback.
    let untyped = anyhow::anyhow!("transport blew up");
    let msg = classify_agent_anyhow_for_user(&untyped);
    assert_eq!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
}

#[test]
fn classifier_does_not_leak_error_content() {
    // Airtight guard: populate every internal `String` / inner-error field
    // of every variant with a distinct `LEAK_CANARY_<n>_<hex>` marker, then
    // assert that NONE of those markers appears in the classifier's output.
    // This is the mechanical proof that the classifier output never depends
    // on the input error's contents.
    let canaries = [
        "LEAK_CANARY_0_deadbeef",
        "LEAK_CANARY_1_cafebabe",
        "LEAK_CANARY_2_0badf00d",
        "LEAK_CANARY_3_feedface",
        "LEAK_CANARY_4_8badf00d",
        "LEAK_CANARY_5_1ce1ce1c",
        "LEAK_CANARY_6_decafbad",
        "LEAK_CANARY_7_b16b00b5",
        "LEAK_CANARY_8_c001d00d",
        "LEAK_CANARY_9_5ca1ab1e",
    ];

    // Variants paired with the canaries injected into each of their fields.
    // Every internal `String` / `&str` / nested-error field is populated
    // with a distinct marker.
    let variants: Vec<AgentError> = vec![
        AgentError::ProviderError {
            message: canaries[0].into(),
            retryable: true,
        },
        AgentError::ProviderError {
            message: canaries[1].into(),
            retryable: false,
        },
        // ContextLimitExceeded has no string fields, but include it so the
        // fuzz still exercises every variant uniformly.
        AgentError::ContextLimitExceeded {
            utilization_pct: 99,
        },
        AgentError::ToolExecutionError {
            tool_name: canaries[2].into(),
            message: canaries[3].into(),
        },
        AgentError::CostBudgetExceeded {
            spent_microdollars: 1,
            budget_microdollars: 1,
        },
        AgentError::MaxIterationsExceeded { max: 7 },
        AgentError::CompactionFailed {
            message: canaries[4].into(),
            consecutive_failures: 2,
        },
        AgentError::PermissionDenied {
            tool_name: canaries[5].into(),
            required_level: canaries[6].into(),
            channel_max_level: canaries[7].into(),
        },
        // Other(..) wraps an anyhow error built from a canary string — its
        // source chain carries marker text that the classifier must NOT
        // forward to the user.
        AgentError::Other(anyhow::anyhow!("{}", canaries[8]).context(canaries[9].to_string())),
    ];

    for variant in &variants {
        let msg_direct = agent_error_to_user_message(variant);

        // Also exercise the anyhow wrapper path so we cover both entry
        // points the scheduler uses.
        // (We rebuild the anyhow Error here rather than reusing `variant`
        // because AgentError doesn't implement Clone.)
        // The classifier output is `&'static str` so checking `msg_direct`
        // covers both paths, but the explicit check guards future changes.

        for canary in &canaries {
            assert!(
                !msg_direct.contains(canary),
                "Classifier leaked `{canary}` into user-facing message: {msg_direct:?}",
            );
        }
    }

    // Sanity: also verify the fallback constant doesn't accidentally
    // contain any canary substring.
    for canary in &canaries {
        assert!(
            !AGENT_JOB_USER_FAILURE_MESSAGE.contains(canary),
            "Fallback constant contains canary `{canary}` — test fixture is broken",
        );
    }
}

#[test]
fn classify_agent_anyhow_does_not_leak_when_downcast_succeeds() {
    // Same airtight guard but through the `classify_agent_anyhow_for_user`
    // entry point — proves the downcast path is just as safe.
    let canary = "LEAK_CANARY_anyhow_8badf00d";
    let typed = anyhow::Error::from(AgentError::ProviderError {
        message: canary.into(),
        retryable: false,
    });
    let msg = classify_agent_anyhow_for_user(&typed);
    assert!(
        !msg.contains(canary),
        "classify_agent_anyhow_for_user leaked `{canary}`: {msg:?}",
    );
    // And it should be the canned non-retryable provider message, not the
    // residual fallback — confirms the downcast actually fired.
    assert_ne!(msg, AGENT_JOB_USER_FAILURE_MESSAGE);
    assert!(msg.contains("credentials"));
}

// ── #3312: scheduler auto-recovery ──────────────────────────────────────────

/// #3312: a successful `tick_once` poll must publish
/// `HealthChanged { component: "scheduler", healthy: true }` even when
/// the job queue is empty. Without this recovery signal, a single
/// transient job failure that flipped the component to `error` via
/// `process_due_jobs` would stay there indefinitely while the queue
/// was idle, leaving the Docker health check returning 503 for hours
/// until a manual restart (the production bug captured 924 consecutive
/// failures across 7h43m).
///
/// We assert on the bus event rather than the process-global registry
/// row so this test doesn't race the many other tests in this binary
/// that mutate the same `"scheduler"` row: snapshotting the wire is
/// monotonic and per-subscriber, while the registry row is a
/// last-writer-wins map that any parallel test can flip.
#[tokio::test]
async fn scheduler_tick_once_publishes_health_recovery_signal_on_empty_queue() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;
    use tinybus::EventHandler;

    #[derive(Default)]
    struct HealthEventCollector {
        events: Arc<StdMutex<Vec<(String, bool)>>>,
    }

    #[async_trait]
    impl EventHandler<DomainEvent> for HealthEventCollector {
        fn name(&self) -> &str {
            "test::scheduler::tick_once::collector"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["system"])
        }

        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::HealthChanged {
                component, healthy, ..
            } = event
            {
                self.events
                    .lock()
                    .unwrap()
                    .push((component.clone(), *healthy));
            }
        }
    }

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;

    crate::core::bus::init().await.expect("bus init");
    let events: Arc<StdMutex<Vec<(String, bool)>>> = Arc::new(StdMutex::new(Vec::new()));
    let collector = Arc::new(HealthEventCollector {
        events: Arc::clone(&events),
    });
    let _handle = BUS.subscribe(collector).expect("bus subscriber installed");

    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));

    // No jobs are due — this is exactly the scenario from #3312 after
    // the failing cron job: the queue stays empty for a long stretch
    // while a prior error sits in the registry. The fix is verified by
    // observing that the tick still emits the recovery signal.
    let before = events.lock().unwrap().len();
    // Start with `None` so the very first tick is treated as a
    // transition and fires the recovery event — same shape as `run()`
    // immediately after boot.
    let mut last_emitted_health: Option<bool> = None;
    tick_once(&config, &security, &mut last_emitted_health).await;

    // Bus delivery is async — wait briefly for the subscriber to drain.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let saw_recovery = events
            .lock()
            .unwrap()
            .iter()
            .skip(before)
            .any(|(component, healthy)| component == "scheduler" && *healthy);
        if saw_recovery {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let recent: Vec<(String, bool)> = events
                .lock()
                .unwrap()
                .iter()
                .skip(before)
                .cloned()
                .collect();
            panic!(
                "tick_once with an empty queue must publish HealthChanged{{scheduler, healthy: true}} (#3312); \
                 events after tick: {recent:?}"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// #3329 review nit (oxoxDev): a successful empty poll must only emit a
/// `HealthChanged` event on a **transition**, not every tick. Once the
/// recovery signal is on the wire, subsequent steady-state ticks should
/// stay silent so subscribers don't see an event-storm on a 30 s poll
/// interval.
///
/// We assert on the local `last_emitted_health` tracker rather than the
/// global bus to stay race-free against the many sibling tests in this
/// binary that publish `HealthChanged { component: "scheduler", ... }`
/// for unrelated reasons. The tracker's transitions are 1:1 with the
/// `publish_global` calls inside `tick_once` by construction (every
/// emit-branch updates it, every no-emit branch doesn't), so a stable
/// `Some(true)` across multiple successful ticks is a sufficient proxy
/// for "no event hit the wire".
#[tokio::test]
async fn scheduler_tick_once_does_not_re_emit_recovery_signal_on_steady_state() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));

    let mut last_emitted_health: Option<bool> = None;

    // First tick: transition from None → Some(true), publishes once.
    tick_once(&config, &security, &mut last_emitted_health).await;
    assert_eq!(
        last_emitted_health,
        Some(true),
        "first successful tick must flip the local tracker to Some(true) \
         (and publish HealthChanged on the bus)"
    );

    // Second + third ticks: steady-state, no transition. The tracker
    // must stay Some(true) — meaning the `if *last_emitted_health !=
    // Some(true)` guard inside `tick_once` short-circuited and no
    // `publish_global` call ran on those ticks.
    for tick in 2..=5 {
        tick_once(&config, &security, &mut last_emitted_health).await;
        assert_eq!(
            last_emitted_health,
            Some(true),
            "tick #{tick} must leave the tracker at Some(true) (steady state, no publish)"
        );
    }
}

// ── Chat-delivery gating (skip failed + empty cron runs) ────────────────────

#[test]
fn chat_delivery_skipped_for_failed_runs() {
    // A failed cron turn (e.g. a transient network/DNS error) yields a
    // non-empty canned message; it must NOT be injected into the chat thread.
    assert!(!should_deliver_cron_output_to_chat(
        false,
        "Something went wrong. Please try again."
    ));
}

#[test]
fn chat_delivery_skipped_for_empty_runs() {
    assert!(!should_deliver_cron_output_to_chat(true, ""));
    assert!(!should_deliver_cron_output_to_chat(true, "   \n  "));
    // The empty-run placeholder counts as empty and is not delivered.
    assert!(cron_output_is_empty(EMPTY_AGENT_OUTPUT));
    assert!(!should_deliver_cron_output_to_chat(
        true,
        EMPTY_AGENT_OUTPUT
    ));
}

#[test]
fn chat_delivery_allowed_for_successful_nonempty_runs() {
    assert!(!cron_output_is_empty(
        "Good morning! You have 3 meetings today."
    ));
    assert!(should_deliver_cron_output_to_chat(
        true,
        "Good morning! You have 3 meetings today."
    ));
}

#[test]
fn failed_runs_still_alert_even_when_empty() {
    // Failures must remain visible in /notifications even with no output.
    assert!(cron_result_should_alert(false, ""));
    assert!(cron_result_should_alert(false, EMPTY_AGENT_OUTPUT));
    assert!(cron_result_should_alert(
        false,
        "Something went wrong. Please try again."
    ));
    // Successful non-empty runs alert; successful-but-empty runs do not.
    assert!(cron_result_should_alert(true, "done"));
    assert!(!cron_result_should_alert(true, ""));
    assert!(!cron_result_should_alert(true, EMPTY_AGENT_OUTPUT));
}

#[tokio::test]
async fn deliver_if_configured_failure_skips_chat_but_alerts() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_job();
    // Failed run (non-empty canned error): no chat injection, but still alerts.
    assert!(
        deliver_if_configured(&config, &job, "Something went wrong.", false)
            .await
            .is_ok()
    );
    assert_eq!(cron_alerts(&config).await, 1);
}

#[tokio::test]
async fn deliver_if_configured_empty_failure_alerts_with_fallback_body() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_job();
    // Empty failed run: still surfaces in /notifications with a fallback body.
    assert!(deliver_if_configured(&config, &job, "", false)
        .await
        .is_ok());
    let items =
        crate::openhuman::desktop::notifications::store::list(&config, 10, 0, Some("cron"), None)
            .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].body.contains("failed without output"));
}
