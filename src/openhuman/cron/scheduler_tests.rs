use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, ActiveHours, DeliveryConfig};
use crate::openhuman::security::SecurityPolicy;
use chrono::{Duration as ChronoDuration, Timelike, Utc};
use std::sync::Arc;
use tempfile::TempDir;

async fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
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

#[tokio::test]
async fn run_job_command_success() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = test_job("echo scheduler-ok");
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("scheduler-ok"));
    assert!(output.contains("status=exit status: 0"));
}

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
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("definitely_missing_file_for_scheduler_test"));
    assert!(output.contains("status=exit status:"));
}

#[tokio::test]
async fn run_job_command_times_out() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.autonomy.allowed_commands = vec!["sleep".into()];
    // Pin `/bin/sleep` — see note on `run_job_command_failure` for why.
    let job = test_job("/bin/sleep 1");
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

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
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

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
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

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
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

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
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let (success, output) = run_job_command(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("blocked by security policy"));
    assert!(output.contains("rate limit exceeded"));
}

#[tokio::test]
async fn execute_job_with_retry_recovers_after_first_failure() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    config.autonomy.allowed_commands = vec!["sh".into()];
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    // Pin absolute paths inside the script too — some dev
    // environments have a homebrew `touch` on PATH that macOS
    // SIP refuses to execute under an unsigned cargo-test binary.
    tokio::fs::write(
        config.workspace_dir.join("retry-once.sh"),
        "#!/bin/sh\nif [ -f retry-ok.flag ]; then\n  echo recovered\n  exit 0\nfi\n/usr/bin/touch retry-ok.flag\nexit 1\n",
    )
    .await
    .unwrap();
    let job = test_job("/bin/sh ./retry-once.sh");

    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("recovered"));
}

#[tokio::test]
async fn execute_job_with_retry_exhausts_attempts() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 1;
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

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

    let (success, output) = run_agent_job(&config, &job).await;
    assert!(!success, "Agent job without provider key should fail");
    assert!(
        !output.is_empty(),
        "Expected non-empty error output from failed agent job"
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
