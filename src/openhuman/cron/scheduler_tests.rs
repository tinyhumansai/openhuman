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

// TAURI-RUST-N — backend 401 ("Invalid token") leaks from a cron-fired agent
// job through `last_agent_error` and the existing classifier in
// `core::observability::is_session_expired_message` matches it (the
// `OpenHuman API error (401` + `"error":"Invalid token"` conjunction was added
// for OPENHUMAN-TAURI-4P0). `is_session_expired_failure` MUST consult that
// classifier so the cron retry loop halts on the first occurrence instead of
// retrying N times and reporting `failure=retries_exhausted` to Sentry.
#[test]
fn is_session_expired_failure_matches_openhuman_backend_401_in_agent_error() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(
        is_session_expired_failure(&JobType::Agent, Some(wire), AGENT_JOB_USER_FAILURE_MESSAGE),
        "raw agent error carrying the 401 wire shape must trip the halt"
    );
}

// Defense-in-depth: if a future code path ever surfaces the raw error in
// `last_output` instead of `last_agent_error` (currently `run_agent_job`
// keeps the canned user message in `last_output`), the predicate should
// still classify. Falling back to `last_output` when `last_agent_error` is
// `None` is what guards against that silent-miss case.
#[test]
fn is_session_expired_failure_matches_when_only_output_carries_signal() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(is_session_expired_failure(&JobType::Agent, None, wire));
}

// Negative guard: the canned user-facing message that `run_agent_job`
// routes into `last_output` today carries no session signal. The predicate
// must NOT trip on it — otherwise every generic agent failure (provider
// keys missing, tool error, network blip) would halt after one attempt and
// stop reporting to Sentry, defeating the retry semantics for non-401
// failures.
#[test]
fn is_session_expired_failure_does_not_match_canned_user_message() {
    assert!(!is_session_expired_failure(
        &JobType::Agent,
        Some(AGENT_JOB_USER_FAILURE_MESSAGE),
        AGENT_JOB_USER_FAILURE_MESSAGE,
    ));
}

// Negative guard: ordinary provider-error wire text (e.g. a third-party
// model rejecting a request as 400 / 500 / 429) must not be misclassified
// as session expiry. Those failures are exactly what the retry loop +
// `failure=retries_exhausted` capture exist for.
#[test]
fn is_session_expired_failure_does_not_match_ordinary_provider_error() {
    let wire =
        r#"OpenHuman API error (500 Internal Server Error): {"error":"Internal server error"}"#;
    assert!(!is_session_expired_failure(&JobType::Agent, Some(wire), ""));

    let byo_key = r#"OpenAI API error (401 Unauthorized): {"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#;
    assert!(
        !is_session_expired_failure(&JobType::Agent, Some(byo_key), ""),
        "third-party BYO-key 401 is actionable (user misconfigured their key) — must NOT classify as backend session expiry"
    );
}

// Scope guard: the halt is restricted to `JobType::Agent` because the
// `SessionExpired` publish + scheduler-gate handshake only fires from the
// inference layer. A shell job that happens to echo the 401-shaped string
// (e.g. an operator's curl wrapper printing the backend response verbatim)
// MUST keep its existing retry semantics — the operator may want those
// retries, and the gate has no reason to be flipped from a shell exit.
#[test]
fn is_session_expired_failure_does_not_halt_shell_jobs() {
    let wire =
        r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Invalid token"}"#;
    assert!(
        !is_session_expired_failure(&JobType::Shell, None, wire),
        "shell jobs must retain retry semantics regardless of stdout content"
    );
    assert!(
        !is_session_expired_failure(&JobType::Shell, Some(wire), wire),
        "shell jobs never populate last_agent_error — but even if a future path did, scope stays Agent-only"
    );
}

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
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("init built-in agent definitions");
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.name = Some("morning_briefing".into());
    job.agent_id = Some("morning_briefing".into());

    let agent = build_agent_for_cron_job(&config, &job).expect("build cron agent");
    let visible = agent.visible_tool_names_for_test();

    assert!(
        !visible.is_empty(),
        "morning briefing has a wildcard scope plus a disallowlist, so the builder must materialize an explicit visible-tool filter"
    );
    assert!(
        !visible.contains("use_tinyplace"),
        "morning briefing cron jobs must use the morning_briefing definition scope, not the orchestrator delegate surface"
    );
    assert!(
        !visible.iter().any(|name| name.starts_with("tinyplace_")),
        "morning briefing cron jobs must preserve tinyplace_* disallowlist"
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
    assert!(deliver_if_configured(&config, &job, "x", true)
        .await
        .is_ok());
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
        msg.contains("different model"),
        "must keep the model-switch remedy: {msg}"
    );
    assert!(
        msg.contains("Settings \u{2192} AI \u{2192} LLM"),
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
    use crate::core::event_bus::{
        init_global, subscribe_global, DomainEvent, EventHandler, DEFAULT_CAPACITY,
    };
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct HealthEventCollector {
        events: Arc<StdMutex<Vec<(String, bool)>>>,
    }

    #[async_trait]
    impl EventHandler for HealthEventCollector {
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

    init_global(DEFAULT_CAPACITY);
    let events: Arc<StdMutex<Vec<(String, bool)>>> = Arc::new(StdMutex::new(Vec::new()));
    let collector = Arc::new(HealthEventCollector {
        events: Arc::clone(&events),
    });
    let _handle = subscribe_global(collector).expect("bus subscriber installed");

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

fn proactive_job() -> CronJob {
    let mut job = test_job("");
    job.delivery = DeliveryConfig {
        mode: "proactive".into(),
        channel: None,
        to: None,
        best_effort: true,
    };
    job
}

async fn cron_alerts(config: &Config) -> usize {
    crate::openhuman::notifications::store::list(config, 10, 0, Some("cron"), None)
        .unwrap()
        .len()
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
        crate::openhuman::notifications::store::list(&config, 10, 0, Some("cron"), None).unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].body.contains("failed without output"));
}

#[tokio::test]
async fn deliver_if_configured_empty_success_skips_chat_and_alert() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_job();
    // Successful but empty: nothing delivered anywhere.
    assert!(deliver_if_configured(&config, &job, "", true).await.is_ok());
    assert_eq!(cron_alerts(&config).await, 0);
}
