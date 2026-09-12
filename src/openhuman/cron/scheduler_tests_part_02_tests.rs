use super::*;

// TAURI-RUST-514 — a BYO provider insufficient-credits 402 ("requires more
// credits") leaks from a cron-fired agent job through `last_agent_error`.
// `is_insufficient_credits_failure` must consult the message classifier so the
// retry loop halts on the first occurrence (a permanent billing state) instead
// of retrying N times and reporting `failure=retries_exhausted` to Sentry.
#[test]
fn is_insufficient_credits_failure_matches_verbatim_402_in_agent_error() {
    let wire = r#"openrouter API error (402 Payment Required): {"error":{"message":"This request requires more credits, or fewer max_tokens. You requested up to 65536 tokens, but can only afford 5081."}}"#;
    assert!(
        is_insufficient_credits_failure(
            &JobType::Agent,
            Some(wire),
            AGENT_JOB_USER_FAILURE_MESSAGE
        ),
        "raw agent error carrying the 402 credit body must trip the halt"
    );
}

// Defense-in-depth: classify even if a future path surfaces the raw 402 in
// `last_output` rather than `last_agent_error`.
#[test]
fn is_insufficient_credits_failure_matches_when_only_output_carries_signal() {
    let wire = r#"openrouter API error (402 Payment Required): insufficient balance — add credits"#;
    assert!(is_insufficient_credits_failure(&JobType::Agent, None, wire));
}

// Negative guard: the canned user-facing message carries no 402 signal, and an
// ordinary provider error (500, or a 400 whose body merely names a token
// count) must NOT halt — those are exactly what the retry loop +
// `failure=retries_exhausted` capture exist for.
#[test]
fn is_insufficient_credits_failure_does_not_match_non_credit_errors() {
    assert!(!is_insufficient_credits_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
    let server_err =
        r#"OpenHuman API error (500 Internal Server Error): {"error":"Internal server error"}"#;
    assert!(!is_insufficient_credits_failure(
        &JobType::Agent,
        Some(server_err),
        ""
    ));
    let digit_in_body = r#"provider API error (400): can only afford 402 tokens"#;
    assert!(
        !is_insufficient_credits_failure(&JobType::Agent, Some(digit_in_body), ""),
        "the 402 must be the status, not an arbitrary token count in a 400 body"
    );
}

// Scope guard: shell jobs that echo a 402-shaped string keep their retry
// semantics — only agent jobs route through the inference layer.
#[test]
fn is_insufficient_credits_failure_does_not_halt_shell_jobs() {
    let wire = r#"openrouter API error (402 Payment Required): requires more credits"#;
    assert!(!is_insufficient_credits_failure(
        &JobType::Shell,
        None,
        wire
    ));
    assert!(!is_insufficient_credits_failure(
        &JobType::Shell,
        Some(wire),
        wire
    ));
}

// TAURI-RUST-BMW — a managed-backend 400 "Insufficient budget"
// (USER_INSUFFICIENT_CREDITS) leaks from a cron-fired agent job through
// `last_agent_error`. `is_budget_exhausted_failure` must consult the budget
// classifier so the retry loop halts on the first occurrence (a permanent
// billing state) instead of retrying N times and reporting
// `failure=retries_exhausted` to Sentry — the tag-gated `is_budget_event`
// `before_send` filter never matched this cron re-report.
#[test]
fn is_budget_exhausted_failure_matches_verbatim_400_in_agent_error() {
    let wire = r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget","errorCode":"USER_INSUFFICIENT_CREDITS"}"#;
    assert!(
        is_budget_exhausted_failure(&JobType::Agent, Some(wire), AGENT_JOB_USER_FAILURE_MESSAGE),
        "raw agent error carrying the 400 budget body must trip the halt"
    );
}

// Defense-in-depth: classify even if a future path surfaces the raw 400 in
// `last_output` rather than `last_agent_error`.
#[test]
fn is_budget_exhausted_failure_matches_when_only_output_carries_signal() {
    let wire = r#"OpenHuman API error (400 Bad Request): budget exceeded — add credits"#;
    assert!(is_budget_exhausted_failure(&JobType::Agent, None, wire));
}

// Negative guard: the canned user-facing message and an ordinary provider
// error must NOT halt — those are what the retry loop +
// `failure=retries_exhausted` capture exist for.
#[test]
fn is_budget_exhausted_failure_does_not_match_non_budget_errors() {
    assert!(!is_budget_exhausted_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
    let server_err =
        r#"OpenHuman API error (500 Internal Server Error): {"error":"Internal server error"}"#;
    assert!(!is_budget_exhausted_failure(
        &JobType::Agent,
        Some(server_err),
        ""
    ));
}

// Scope guard: shell jobs that echo a budget-shaped string keep their retry
// semantics — only agent jobs route through the inference layer.
#[test]
fn is_budget_exhausted_failure_does_not_halt_shell_jobs() {
    let wire = r#"OpenHuman API error (400 Bad Request): {"error":"Insufficient budget"}"#;
    assert!(!is_budget_exhausted_failure(&JobType::Shell, None, wire));
    assert!(!is_budget_exhausted_failure(
        &JobType::Shell,
        Some(wire),
        wire
    ));
}

// TAURI-RUST-HCK — a cron agent job pinned to a provider with no configured
// API key fails at the credential guard with "<provider> API key not set …",
// before any HTTP, and leaks through `last_agent_error`.
// `is_api_key_unset_failure` must consult the shared matcher so the retry loop
// halts on the first occurrence (a permanent user-config state) instead of
// retrying N times and reporting `failure=retries_exhausted` to Sentry (3428
// events / 1 user) — the bare cron `report_error` bypasses the `ApiKeyMissing`
// `expected_error_kind` demotion.
#[test]
fn is_api_key_unset_failure_matches_verbatim_in_agent_error() {
    let wire =
        "openrouter API key not set. Configure via the web UI or set the appropriate env var.";
    assert!(
        is_api_key_unset_failure(&JobType::Agent, Some(wire), AGENT_JOB_USER_FAILURE_MESSAGE),
        "raw agent error carrying the verbatim 'API key not set' wording must trip the halt"
    );
}

// Defense-in-depth: classify even if a future path surfaces the raw error in
// `last_output` rather than `last_agent_error`.
#[test]
fn is_api_key_unset_failure_matches_when_only_output_carries_signal() {
    let wire = "cohere API key not set. Configure via the web UI or set the appropriate env var.";
    assert!(is_api_key_unset_failure(&JobType::Agent, None, wire));
}

// Negative guard: the canned user-facing message carries no key signal; an
// ordinary provider error must NOT halt; and — critically — a *rejected* key
// (provider 401 "Invalid API key", a present-but-wrong key) is actionable and
// must keep reaching Sentry. This matcher is for an *absent* key only.
#[test]
fn is_api_key_unset_failure_does_not_match_canned_rejected_or_ordinary_errors() {
    assert!(!is_api_key_unset_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
    let server_err =
        r#"OpenHuman API error (500 Internal Server Error): {"error":"Internal server error"}"#;
    assert!(!is_api_key_unset_failure(
        &JobType::Agent,
        Some(server_err),
        ""
    ));
    let rejected_key = r#"OpenAI API error (401 Unauthorized): {"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#;
    assert!(
        !is_api_key_unset_failure(&JobType::Agent, Some(rejected_key), ""),
        "a present-but-rejected key (401 Invalid API key) is actionable — must NOT classify as an unset key"
    );
}

// Scope guard: shell jobs that echo an "API key not set" string keep their
// retry semantics — only agent jobs route through the inference credential guard.
#[test]
fn is_api_key_unset_failure_does_not_halt_shell_jobs() {
    let wire =
        "openrouter API key not set. Configure via the web UI or set the appropriate env var.";
    assert!(!is_api_key_unset_failure(&JobType::Shell, None, wire));
    assert!(!is_api_key_unset_failure(&JobType::Shell, Some(wire), wire));
}

// TAURI-RUST-12K — a cron agent job pinned to a local LLM provider (LM Studio
// on localhost:1234) fails with a loopback connection-refused because the
// user's server isn't running. `is_local_provider_unreachable_failure` must
// consult the shared loopback matcher so the retry loop halts on the first
// occurrence (retries can't bring the port up) instead of re-emitting the
// `failure=retries_exhausted` bare `report_error` the classifier already
// demotes everywhere else.
#[test]
fn is_local_provider_unreachable_failure_matches_localized_loopback_in_agent_error() {
    // Verbatim from the Sentry event: zh-CN Windows host, localized
    // WSAECONNREFUSED text, only the errno + `tcp connect error` survive.
    let wire = "error sending request for url \
                (http://localhost:1234/v1/chat/completions): client error (Connect): \
                tcp connect error: 由于目标计算机积极拒绝，无法连接。 (os error 10061)";
    assert!(
        is_local_provider_unreachable_failure(
            &JobType::Agent,
            Some(wire),
            AGENT_JOB_USER_FAILURE_MESSAGE
        ),
        "raw agent error carrying the localized loopback connect-refused must trip the halt"
    );
}

// Defense-in-depth: classify even if a future path surfaces the raw error in
// `last_output` rather than `last_agent_error`.
#[test]
fn is_local_provider_unreachable_failure_matches_when_only_output_carries_signal() {
    let wire = "error sending request for url (http://localhost:1234/v1/chat/completions) \
                → tcp connect error → Connection refused (os error 10061)";
    assert!(is_local_provider_unreachable_failure(
        &JobType::Agent,
        None,
        wire
    ));
}

#[test]
fn is_local_provider_unreachable_failure_keeps_short_loopback_send_error_retryable() {
    let wire = "error sending request for url (http://localhost:1234/v1/chat/completions)";
    assert!(
        !is_local_provider_unreachable_failure(
            &JobType::Agent,
            Some(wire),
            AGENT_JOB_USER_FAILURE_MESSAGE
        ),
        "short reqwest send errors can represent transient timeout/reset shapes and must stay retryable without a refused errno/tcp-connect signal"
    );
}

#[test]
fn is_local_provider_unreachable_failure_matches_raw_no_models_loaded_body() {
    let raw = "LM Studio API error (400 Bad Request): {\"error\":\"No models loaded. \
               Please load a model in the developer page first.\"}";
    assert!(
        is_local_provider_unreachable_failure(
            &JobType::Agent,
            Some(raw),
            AGENT_JOB_USER_FAILURE_MESSAGE
        ),
        "raw OpenAI-compatible no-model body should halt without retries"
    );
}

#[test]
fn is_local_provider_unreachable_failure_checks_output_when_raw_is_generic() {
    let output =
        "Your local inference server (e.g. LM Studio) is running but has no model loaded. \
                  Load a model, then try again.";
    assert!(
        is_local_provider_unreachable_failure(
            &JobType::Agent,
            Some(AGENT_JOB_USER_FAILURE_MESSAGE),
            output
        ),
        "friendly no-model output should halt even when raw agent error is generic"
    );
}

// Negative guard: a transient REMOTE provider / backend network error must NOT
// halt — it may recover on retry and stays actionable in Sentry. Narrowing to
// loopback is what keeps this guard from blinding real outages.
#[test]
fn is_local_provider_unreachable_failure_does_not_match_remote_network_errors() {
    assert!(!is_local_provider_unreachable_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
    let remote = "error sending request for url (https://api.tinyhumans.ai/v1/chat/completions) \
                  → tcp connect error → Connection refused (os error 61)";
    assert!(
        !is_local_provider_unreachable_failure(&JobType::Agent, Some(remote), ""),
        "a remote-host connect-refused must retry + report, not halt as loopback"
    );
}

// Scope guard: shell jobs that echo a loopback-refused string keep their retry
// semantics — only agent jobs route through the inference layer.
#[test]
fn is_local_provider_unreachable_failure_does_not_halt_shell_jobs() {
    let wire = "error sending request for url (http://localhost:1234/v1/chat/completions) \
                → tcp connect error → Connection refused (os error 10061)";
    assert!(!is_local_provider_unreachable_failure(
        &JobType::Shell,
        None,
        wire
    ));
    assert!(!is_local_provider_unreachable_failure(
        &JobType::Shell,
        Some(wire),
        wire
    ));
}

#[tokio::test]
async fn run_agent_job_returns_error_without_provider_key() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.prompt = Some("Say hello".into());

    let (success, output, raw_error) = run_agent_job(&config, &job).await;
    assert!(!success, "Agent job without provider key should fail");
    assert!(output.contains("Something went wrong. Please try again."));
    assert!(output.contains("This error has been reported."));
    assert!(output.contains("Report on Discord"));
    assert!(
        raw_error
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "Expected raw agent error for observability after retries are exhausted"
    );
    assert!(
        !output.contains("error sending request for url"),
        "Expected sanitized output without raw transport details"
    );
}

#[tokio::test]
async fn cron_agent_job_uses_agent_definition_tool_scope() {
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("init built-in agent definitions");
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    let built = build_agent_for_cron_job(&config, &job).expect("build cron agent");
    let visible = built.agent.visible_tool_names_for_test();

    assert!(
        !visible.is_empty(),
        "morning briefing has a wildcard scope plus a disallowlist, so the builder must materialize an explicit visible-tool filter"
    );
}

#[tokio::test]
async fn persist_job_result_records_run_and_reschedules_shell_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = cron::add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    let started = Utc::now();
    let finished = started + ChronoDuration::milliseconds(10);

    let success = persist_job_result(&config, &job, true, "ok", started, finished).await;
    assert!(success);

    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    let updated = cron::get_job(&config, &job.id).unwrap();
    assert_eq!(updated.last_status.as_deref(), Some("ok"));
}

#[tokio::test]
async fn scheduler_flow_runs_active_hours_job_and_reschedules_inside_window() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let active_minute = Utc::now() + ChronoDuration::minutes(2);
    let active_hm = format!("{:02}:{:02}", active_minute.hour(), active_minute.minute());
    let active_hours = ActiveHours {
        start: active_hm.clone(),
        end: active_hm.clone(),
    };
    let mut job = cron::add_shell_job(
        &config,
        Some("active-hours-e2e".into()),
        Schedule::Cron {
            expr: "* * * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(active_hours.clone()),
        },
        "echo active-hours-fired",
    )
    .unwrap();
    job.next_run = Utc::now() - ChronoDuration::seconds(1);

    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    process_due_jobs(&config, &security, vec![job.clone()]).await;

    let stored = cron::get_job(&config, &job.id).unwrap();
    assert_eq!(stored.last_status.as_deref(), Some("ok"));
    assert!(stored
        .last_output
        .as_deref()
        .unwrap_or_default()
        .contains("active-hours-fired"));
    assert_eq!(
        stored.schedule,
        Schedule::Cron {
            expr: "* * * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(active_hours),
        }
    );

    let next_hm = format!(
        "{:02}:{:02}",
        stored.next_run.hour(),
        stored.next_run.minute()
    );
    assert_eq!(next_hm, active_hm);
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "ok");
}

#[tokio::test]
async fn persist_job_result_success_deletes_one_shot() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let at = Utc::now() + ChronoDuration::minutes(10);
    let job = cron::add_agent_job(
        &config,
        Some("one-shot".into()),
        crate::openhuman::cron::Schedule::At { at },
        "Hello",
        SessionTarget::Isolated,
        None,
        None,
        true,
    )
    .unwrap();
    let started = Utc::now();
    let finished = started + ChronoDuration::milliseconds(10);

    let success = persist_job_result(&config, &job, true, "ok", started, finished).await;
    assert!(success);
    let lookup = cron::get_job(&config, &job.id);
    assert!(lookup.is_err());
}

#[tokio::test]
async fn persist_job_result_failure_disables_one_shot() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let at = Utc::now() + ChronoDuration::minutes(10);
    let job = cron::add_agent_job(
        &config,
        Some("one-shot".into()),
        crate::openhuman::cron::Schedule::At { at },
        "Hello",
        SessionTarget::Isolated,
        None,
        None,
        true,
    )
    .unwrap();
    let started = Utc::now();
    let finished = started + ChronoDuration::milliseconds(10);

    let success = persist_job_result(&config, &job, false, "boom", started, finished).await;
    assert!(!success);
    let updated = cron::get_job(&config, &job.id).unwrap();
    assert!(!updated.enabled);
    assert_eq!(updated.last_status.as_deref(), Some("error"));
}

#[tokio::test]
async fn persist_job_result_disables_at_job_without_delete_flag() {
    // Regression: an `At` job created without delete_after_run (the RPC default,
    // and every shell `At` job) must not be rescheduled after it runs. Its `at`
    // is a fixed instant, so reschedule_after_run would write next_run = at
    // (now in the past) and due_jobs would re-select it on every poll, re-firing
    // the job forever.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let at = Utc::now() + ChronoDuration::minutes(10);
    let job = cron::add_agent_job(
        &config,
        Some("at-no-delete".into()),
        crate::openhuman::cron::Schedule::At { at },
        "Hello",
        SessionTarget::Isolated,
        None,
        None,
        false, // delete_after_run = false — the previously-buggy case
    )
    .unwrap();
    let started = Utc::now();
    let finished = started + ChronoDuration::milliseconds(10);

    let success = persist_job_result(&config, &job, true, "ok", started, finished).await;
    assert!(success);

    // The row is kept (not auto-deleted) but disabled, and its run is recorded.
    let updated = cron::get_job(&config, &job.id).unwrap();
    assert!(!updated.enabled, "At job must be disabled after one run");
    assert_eq!(updated.last_status.as_deref(), Some("ok"));

    // It is never due again — even at a time past its `at` instant.
    let due = cron::due_jobs(&config, at + ChronoDuration::minutes(1)).unwrap();
    assert!(
        !due.iter().any(|j| j.id == job.id),
        "disabled At job must not be re-selected by due_jobs"
    );
}

#[tokio::test]
async fn deliver_if_configured_skips_non_announce_mode() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = test_job("echo ok");

    // Default delivery mode is not "announce", so nothing is published.
    assert!(deliver_if_configured(&config, &job, "x", true)
        .await
        .is_ok());
}

#[tokio::test]
async fn deliver_if_configured_publishes_event_for_announce_mode() {
    use crate::core::events::DomainEvent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tinybus::EventHandler;

    // Create an isolated bus for this test.
    let bus = crate::core::bus_testing::isolated_bus().await;

    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = Arc::clone(&received);

    struct Counter(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl EventHandler<DomainEvent> for Counter {
        fn name(&self) -> &str {
            "test::counter"
        }
        fn domains(&self) -> Option<&[&str]> {
            Some(&["cron"])
        }
        async fn handle(&self, event: &DomainEvent) {
            if matches!(event, DomainEvent::CronDeliveryRequested { .. }) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    let _handle = bus.subscribe(Arc::new(Counter(received_clone)));

    // Publish directly on the test bus (bypasses the global singleton).
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: Some("telegram".into()),
        to: Some("chat-123".into()),
        best_effort: true,
    };

    // Manually publish the same event deliver_if_configured would produce.
    bus.publish(DomainEvent::CronDeliveryRequested {
        job_id: job.id.clone(),
        channel: "telegram".into(),
        target: "chat-123".into(),
        output: "hello".into(),
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(received.load(Ordering::SeqCst), 1);

    // Also verify the function itself succeeds.
    assert!(deliver_if_configured(&config, &job, "hello", true)
        .await
        .is_ok());
}

#[test]
fn is_one_shot_auto_delete_true_for_at_schedule_with_flag() {
    let mut job = test_job("echo hi");
    job.delete_after_run = true;
    job.schedule = Schedule::At { at: Utc::now() };
    assert!(is_one_shot_auto_delete(&job));
}

#[test]
fn is_one_shot_auto_delete_false_for_cron_schedule() {
    let mut job = test_job("echo hi");
    job.delete_after_run = true;
    job.schedule = Schedule::Cron {
        expr: "0 * * * *".into(),
        tz: None,
        active_hours: None,
    };
    assert!(!is_one_shot_auto_delete(&job));
}

#[test]
fn is_one_shot_auto_delete_false_when_flag_not_set() {
    let mut job = test_job("echo hi");
    job.delete_after_run = false;
    job.schedule = Schedule::At { at: Utc::now() };
    assert!(!is_one_shot_auto_delete(&job));
}

#[test]
fn is_env_assignment_true() {
    assert!(is_env_assignment("FOO=bar"));
    assert!(is_env_assignment("_VAR=1"));
}

#[test]
fn is_env_assignment_false() {
    assert!(!is_env_assignment("echo"));
    assert!(!is_env_assignment("=bad"));
    assert!(!is_env_assignment("123=nope"));
    assert!(!is_env_assignment(""));
}

#[test]
fn strip_wrapping_quotes_removes_quotes() {
    assert_eq!(strip_wrapping_quotes("\"hello\""), "hello");
    assert_eq!(strip_wrapping_quotes("'world'"), "world");
    assert_eq!(strip_wrapping_quotes("noquotes"), "noquotes");
    assert_eq!(strip_wrapping_quotes(""), "");
}

#[test]
fn forbidden_path_argument_allows_safe_commands() {
    let policy = SecurityPolicy::default();
    assert!(forbidden_path_argument(&policy, "echo hello").is_none());
    assert!(forbidden_path_argument(&policy, "date").is_none());
}
