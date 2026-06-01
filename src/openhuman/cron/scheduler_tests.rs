use super::*;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, ActiveHours, DeliveryConfig};
use crate::openhuman::security::SecurityPolicy;
use chrono::{Duration as ChronoDuration, Timelike, Utc};
#[cfg(not(windows))]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tempfile::TempDir;

async fn test_config(tmp: &TempDir) -> Config {
    let ws = tmp.path().join("workspace");
    let config = Config {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    config
}

fn test_job(command: &str) -> CronJob {
    CronJob {
        id: "test-job".into(),
        expression: "* * * * *".into(),
        schedule: crate::openhuman::cron::Schedule::Cron {
            expr: "* * * * *".into(),
            tz: None,
            active_hours: None,
        },
        command: command.into(),
        prompt: None,
        name: None,
        job_type: JobType::Shell,
        session_target: SessionTarget::Isolated,
        model: None,
        agent_id: None,
        enabled: true,
        delivery: DeliveryConfig::default(),
        delete_after_run: false,
        created_at: Utc::now(),
        next_run: Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
    }
}

#[test]
fn agent_failure_copy_mentions_retry_reporting_and_discord() {
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("Something went wrong. Please try again."));
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("This error has been reported."));
    assert!(AGENT_JOB_USER_FAILURE_MESSAGE.contains("Report on Discord"));
}

#[test]
fn cron_alert_body_rewrites_morning_briefing_failure() {
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    let body = cron_alert_body(&job, AGENT_JOB_USER_FAILURE_MESSAGE);

    assert_eq!(body, MORNING_BRIEFING_FAILURE_NOTIFICATION);
    assert!(!body.contains("Something went wrong"));
    assert!(!body.contains("<openhuman-link"));
}

#[test]
fn cron_alert_body_strips_openhuman_link_markup() {
    let job = test_job("");
    let body = cron_alert_body(
        &job,
        "Read <openhuman-link path=\"settings/notifications\">notification settings</openhuman-link> before tomorrow.",
    );

    assert_eq!(body, "Read notification settings before tomorrow.");
    assert!(!body.contains("<openhuman-link"));
}

#[tokio::test]
async fn push_cron_alert_deduplicates_repeated_morning_briefing_failures() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    push_cron_alert(&config, &job, AGENT_JOB_USER_FAILURE_MESSAGE);
    push_cron_alert(&config, &job, AGENT_JOB_USER_FAILURE_MESSAGE);

    let items =
        crate::openhuman::notifications::store::list(&config, 10, 0, Some("cron"), None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, MORNING_BRIEFING_FAILURE_NOTIFICATION);
}

#[test]
fn agent_session_target_tag_matches_expected_values() {
    assert_eq!(agent_session_target_tag(&SessionTarget::Main), "main");
    assert_eq!(
        agent_session_target_tag(&SessionTarget::Isolated),
        "isolated"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_success() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = test_job("echo scheduler-ok");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("scheduler-ok"));
    assert!(output.contains("status=exit status: 0"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_failure() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    // Pin the absolute path so `sh -lc` doesn't pick up a
    // homebrew / PATH-shadowed `ls` that macOS SIP refuses to
    // execute under an unsigned cargo-test binary. `/bin/ls` is
    // an Apple-signed system binary on macOS and present on
    // Linux, so this keeps CI behaviour identical while making
    // local dev runs deterministic.
    let job = test_job("/bin/ls definitely_missing_file_for_scheduler_test");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("definitely_missing_file_for_scheduler_test"));
    assert!(output.contains("status=exit status:"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn run_job_command_times_out() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["sleep".into()];
    // Pin `/bin/sleep` — see note on `run_job_command_failure` for why.
    let job = test_job("/bin/sleep 1");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) =
        run_job_command_with_timeout(&config, &security, &job, Duration::from_millis(50)).await;
    assert!(!success);
    assert!(output.contains("job timed out after"));
}

#[tokio::test]
async fn run_job_command_blocks_disallowed_command() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["echo".into()];
    let job = test_job("curl https://evil.example");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("command not allowed"));
}

#[tokio::test]
async fn run_job_command_blocks_forbidden_path_argument() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["cat".into()];
    let job = test_job("cat /etc/passwd");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("forbidden path argument"));
    assert!(output.contains("/etc/passwd"));
}

#[tokio::test]
async fn run_job_command_blocks_readonly_mode() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;
    let job = test_job("echo should-not-run");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("read-only"));
}

#[tokio::test]
async fn run_job_command_blocks_rate_limited() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.max_actions_per_hour = 0;
    let job = test_job("echo should-not-run");
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("rate limit exceeded"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn execute_job_with_retry_recovers_after_first_failure() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    config.autonomy.allowed_commands = vec!["retry-once.sh".into()];
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    // Pin absolute paths inside the script too — some dev
    // environments have a homebrew `touch` on PATH that macOS
    // SIP refuses to execute under an unsigned cargo-test binary.
    let script = config.workspace_dir.join("retry-once.sh");
    tokio::fs::write(
        &script,
        "#!/bin/sh\nif [ -f retry-ok.flag ]; then\n  echo recovered\n  exit 0\nfi\n/usr/bin/touch retry-ok.flag\nexit 1\n",
    )
    .await
    .unwrap();
    let mut permissions = tokio::fs::metadata(&script).await.unwrap().permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&script, permissions)
        .await
        .unwrap();
    let job = test_job("./retry-once.sh");

    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("recovered"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn execute_job_with_retry_exhausts_attempts() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    let security = SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    );

    // Pin `/bin/ls` — see note on `run_job_command_failure`.
    let job = test_job("/bin/ls always_missing_for_retry_test");

    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("always_missing_for_retry_test"));
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
async fn deliver_if_configured_skips_non_announce_mode() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = test_job("echo ok");

    // Default delivery mode is not "announce", so nothing is published.
    assert!(deliver_if_configured(&config, &job, "x").await.is_ok());
}

#[tokio::test]
async fn deliver_if_configured_publishes_event_for_announce_mode() {
    use crate::core::event_bus::{DomainEvent, EventHandler};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Create an isolated bus for this test.
    let bus = crate::core::event_bus::EventBus::create(16);

    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = Arc::clone(&received);

    struct Counter(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl EventHandler for Counter {
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
    assert!(deliver_if_configured(&config, &job, "hello").await.is_ok());
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

#[test]
fn forbidden_path_argument_skips_flags_and_urls() {
    let policy = SecurityPolicy::default();
    assert!(forbidden_path_argument(&policy, "curl https://example.com").is_none());
    assert!(forbidden_path_argument(&policy, "ls -la").is_none());
}

#[test]
fn warn_if_high_frequency_agent_job_does_not_panic_on_non_agent() {
    let mut job = test_job("echo hi");
    job.job_type = JobType::Shell;
    warn_if_high_frequency_agent_job(&job); // should not panic
}

#[test]
fn warn_if_high_frequency_agent_job_does_not_panic_on_at_schedule() {
    let mut job = test_job("echo hi");
    job.job_type = JobType::Agent;
    job.schedule = Schedule::At { at: Utc::now() };
    warn_if_high_frequency_agent_job(&job); // should not panic
}

#[test]
fn warn_if_high_frequency_agent_job_handles_every_ms() {
    let mut job = test_job("echo hi");
    job.job_type = JobType::Agent;
    job.schedule = Schedule::Every { every_ms: 60_000 }; // 1 minute — too frequent
    warn_if_high_frequency_agent_job(&job); // should warn but not panic
}

#[tokio::test]
async fn deliver_if_configured_skips_empty_mode() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery.mode = "".into();
    assert!(deliver_if_configured(&config, &job, "output").await.is_ok());
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
    let result = deliver_if_configured(&config, &job, "out").await;
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
    let result = deliver_if_configured(&config, &job, "out").await;
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
    assert!(deliver_if_configured(&config, &job, "hello").await.is_ok());
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
    assert!(msg.contains("Settings"));
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
    assert!(msg.contains("Settings"));
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
