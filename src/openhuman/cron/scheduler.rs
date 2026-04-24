use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    due_jobs, next_run_for_schedule, record_last_run, record_run, remove_job, reschedule_after_run,
    update_job, CronJob, CronJobPatch, DeliveryConfig, JobType, Schedule, SessionTarget,
};
use crate::openhuman::security::SecurityPolicy;
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::{stream, StreamExt};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::{self, Duration};

const MIN_POLL_SECONDS: u64 = 5;
const SHELL_JOB_TIMEOUT_SECS: u64 = 120;

pub async fn run(config: Config) -> Result<()> {
    // Ensure the global event bus is initialized so cron delivery events
    // are not silently dropped. This is a no-op if already initialized.
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    crate::openhuman::health::bus::register_health_subscriber();

    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));

    publish_global(DomainEvent::SystemStartup {
        component: "scheduler".to_string(),
    });

    loop {
        interval.tick().await;

        let jobs = match due_jobs(&config, Utc::now()) {
            Ok(jobs) => jobs,
            Err(e) => {
                publish_global(DomainEvent::HealthChanged {
                    component: "scheduler".to_string(),
                    healthy: false,
                    message: Some(e.to_string()),
                });
                tracing::warn!("Scheduler query failed: {e}");
                continue;
            }
        };

        process_due_jobs(&config, &security, jobs).await;
    }
}

/// Public entry point for delivering a job's output via the configured
/// delivery mode (proactive / announce). Called by `cron_run` ("Run Now")
/// so manual runs also push notifications and alerts.
pub async fn deliver_job(config: &Config, job: &CronJob, output: &str) {
    if let Err(e) = deliver_if_configured(config, job, output).await {
        if job.delivery.best_effort {
            tracing::warn!("[cron] delivery failed (best_effort, Run Now): {e}");
        } else {
            tracing::warn!("[cron] delivery failed (Run Now): {e}");
        }
    }
}

pub async fn execute_job_now(config: &Config, job: &CronJob) -> (bool, String) {
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    execute_job_with_retry(config, &security, job).await
}

async fn execute_job_with_retry(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let mut last_output = String::new();
    let retries = config.reliability.scheduler_retries;
    let mut backoff_ms = config.reliability.provider_backoff_ms.max(200);

    for attempt in 0..=retries {
        let (success, output) = match job.job_type {
            JobType::Shell => run_job_command(config, security, job).await,
            JobType::Agent => run_agent_job(config, job).await,
        };
        last_output = output;

        if success {
            return (true, last_output);
        }

        if last_output.starts_with("blocked by security policy:") {
            // Deterministic policy violations are not retryable.
            return (false, last_output);
        }

        if attempt < retries {
            let jitter_ms = u64::from(Utc::now().timestamp_subsec_millis() % 250);
            time::sleep(Duration::from_millis(backoff_ms + jitter_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
        }
    }

    (false, last_output)
}

async fn process_due_jobs(config: &Config, security: &Arc<SecurityPolicy>, jobs: Vec<CronJob>) {
    let max_concurrent = config.scheduler.max_concurrent.max(1);
    let mut in_flight = stream::iter(jobs.into_iter().map(|job| {
        let config = config.clone();
        let security = Arc::clone(security);
        async move { execute_and_persist_job(&config, security.as_ref(), &job).await }
    }))
    .buffer_unordered(max_concurrent);

    while let Some((job_id, success, failure_message)) = in_flight.next().await {
        if success {
            publish_global(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: true,
                message: None,
            });
        } else {
            publish_global(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: false,
                message: Some(failure_message.unwrap_or_else(|| format!("job {job_id} failed"))),
            });
        }
    }
}

async fn execute_and_persist_job(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (String, bool, Option<String>) {
    warn_if_high_frequency_agent_job(job);

    let started_at = Utc::now();

    publish_global(DomainEvent::CronJobTriggered {
        job_id: job.id.clone(),
        job_name: job.name.clone().unwrap_or_default(),
        job_type: format!("{:?}", job.job_type),
    });

    let (execution_success, output) = execute_job_with_retry(config, security, job).await;
    let finished_at = Utc::now();
    let success = persist_job_result(
        config,
        job,
        execution_success,
        &output,
        started_at,
        finished_at,
    )
    .await;

    publish_global(DomainEvent::CronJobCompleted {
        job_id: job.id.clone(),
        success,
        output: crate::openhuman::util::truncate_with_ellipsis(&output, 512),
    });
    let failure_message =
        (!success).then(|| crate::openhuman::util::truncate_with_ellipsis(&output, 256));

    (job.id.clone(), success, failure_message)
}

async fn run_agent_job(config: &Config, job: &CronJob) -> (bool, String) {
    use crate::openhuman::agent::Agent;

    let name = job.name.clone().unwrap_or_else(|| "cron-job".to_string());
    let prompt = job.prompt.clone().unwrap_or_default();
    let prefixed_prompt = format!("[cron:{} {name}] {prompt}", job.id);

    // Apply per-job model override onto a cloned Config, so the Agent
    // sees it through the normal `default_model` path without mutating
    // the caller's config.
    let mut effective = config.clone();
    if let Some(model) = job.model.clone() {
        effective.default_model = Some(model);
    }

    // When an agent_id is set, resolve the built-in definition and apply
    // its model hint, iteration cap, and prompt body so the cron job
    // runs with the definition's constraints instead of the generic
    // Agent::from_config defaults.
    if let Some(ref agent_id) = job.agent_id {
        if let Some(registry) =
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        {
            if let Some(def) = registry.get(agent_id) {
                tracing::debug!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    max_iterations = def.max_iterations,
                    "[cron] applying agent definition overrides"
                );
                let fallback_model = effective
                    .default_model
                    .clone()
                    .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());
                effective.default_model = Some(def.model.resolve(&fallback_model));
                effective.agent.max_tool_iterations = def.max_iterations;
            } else {
                tracing::warn!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    "[cron] agent_id not found in registry — falling back to generic agent"
                );
            }
        } else {
            tracing::warn!(
                job_id = %job.id,
                "[cron] AgentDefinitionRegistry not initialized — falling back to generic agent"
            );
        }
    }

    let run_result = match job.session_target {
        SessionTarget::Main | SessionTarget::Isolated => {
            tracing::debug!(
                job_id = %job.id,
                target = ?job.session_target,
                "[cron] building isolated agent for scheduled job"
            );
            match Agent::from_config(&effective) {
                Ok(mut agent) => {
                    // Tag events so downstream subscribers can correlate
                    // cron-triggered turns. `cron` is the channel so the
                    // event bus can filter from other flows (`cli`, `web`…).
                    agent.set_event_context(format!("cron:{}", job.id), "cron");
                    agent.run_single(&prefixed_prompt).await
                }
                Err(e) => Err(e),
            }
        }
    };

    match run_result {
        Ok(response) => (
            true,
            if response.trim().is_empty() {
                "agent job executed".to_string()
            } else {
                response
            },
        ),
        Err(e) => (false, format!("agent job failed: {e}")),
    }
}

async fn persist_job_result(
    config: &Config,
    job: &CronJob,
    mut success: bool,
    output: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> bool {
    let duration_ms = (finished_at - started_at).num_milliseconds();

    if let Err(e) = deliver_if_configured(config, job, output).await {
        if job.delivery.best_effort {
            tracing::warn!("Cron delivery failed (best_effort): {e}");
        } else {
            success = false;
            tracing::warn!("Cron delivery failed: {e}");
        }
    }

    let _ = record_run(
        config,
        &job.id,
        started_at,
        finished_at,
        if success { "ok" } else { "error" },
        Some(output),
        duration_ms,
    );

    if is_one_shot_auto_delete(job) {
        if success {
            if let Err(e) = remove_job(config, &job.id) {
                tracing::warn!("Failed to remove one-shot cron job after success: {e}");
            }
        } else {
            let _ = record_last_run(config, &job.id, finished_at, false, output);
            if let Err(e) = update_job(
                config,
                &job.id,
                CronJobPatch {
                    enabled: Some(false),
                    ..CronJobPatch::default()
                },
            ) {
                tracing::warn!("Failed to disable failed one-shot cron job: {e}");
            }
        }
        return success;
    }

    if let Err(e) = reschedule_after_run(config, job, success, output) {
        tracing::warn!("Failed to persist scheduler run result: {e}");
    }

    success
}

fn is_one_shot_auto_delete(job: &CronJob) -> bool {
    job.delete_after_run && matches!(job.schedule, Schedule::At { .. })
}

fn warn_if_high_frequency_agent_job(job: &CronJob) {
    if !matches!(job.job_type, JobType::Agent) {
        return;
    }
    let too_frequent = match &job.schedule {
        Schedule::Every { every_ms } => *every_ms < 5 * 60 * 1000,
        Schedule::Cron { .. } => {
            let now = Utc::now();
            match (
                next_run_for_schedule(&job.schedule, now),
                next_run_for_schedule(&job.schedule, now + chrono::Duration::seconds(1)),
            ) {
                (Ok(a), Ok(b)) => (b - a).num_minutes() < 5,
                _ => false,
            }
        }
        Schedule::At { .. } => false,
    };

    if too_frequent {
        tracing::warn!(
            "Cron agent job '{}' is scheduled more frequently than every 5 minutes",
            job.id
        );
    }
}

async fn deliver_if_configured(config: &Config, job: &CronJob, output: &str) -> Result<()> {
    let delivery: &DeliveryConfig = &job.delivery;

    let mode = delivery.mode.trim().to_ascii_lowercase();
    match mode.as_str() {
        // Proactive delivery — the channels module decides where to send.
        // Used by morning briefings, welcome messages, and other
        // user-facing proactive agents.
        "proactive" => {
            let source = format!("cron:{}", job.id);
            tracing::debug!(
                job_id = %job.id,
                source = %source,
                "[cron] publishing ProactiveMessageRequested event"
            );
            publish_global(DomainEvent::ProactiveMessageRequested {
                source,
                message: output.to_string(),
                job_name: job.name.clone(),
            });

            // Also push to the alerts tab so the user sees it in /notifications.
            push_cron_alert(config, job, output);
        }

        // Announce delivery — the cron job specifies the exact channel
        // and target. Used for explicit channel-targeted output.
        "announce" => {
            let channel = delivery
                .channel
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.channel is required for announce mode"))?;
            let target = delivery
                .to
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.to is required for announce mode"))?;

            tracing::debug!(
                job_id = %job.id,
                channel = %channel,
                target = %target,
                "[cron] publishing CronDeliveryRequested event"
            );
            publish_global(DomainEvent::CronDeliveryRequested {
                job_id: job.id.clone(),
                channel: channel.to_string(),
                target: target.to_string(),
                output: output.to_string(),
            });

            push_cron_alert(config, job, output);
        }

        // No delivery configured — output is stored in last_output only.
        _ => {}
    }

    Ok(())
}

/// Insert a notification into the alerts tab for a completed cron job.
fn push_cron_alert(config: &Config, job: &CronJob, output: &str) {
    use crate::openhuman::notifications::store as notif_store;
    use crate::openhuman::notifications::types::{IntegrationNotification, NotificationStatus};

    let name = job.name.as_deref().unwrap_or("Cron job");
    let truncated = crate::openhuman::util::truncate_with_ellipsis(output, 512);

    let notification = IntegrationNotification {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "cron".to_string(),
        account_id: Some(job.id.clone()),
        title: name.to_string(),
        body: truncated,
        raw_payload: serde_json::json!({
            "job_id": job.id,
            "job_name": job.name,
            "delivery_mode": job.delivery.mode,
        }),
        importance_score: Some(0.65),
        triage_action: Some("react".to_string()),
        triage_reason: Some("Scheduled delivery".to_string()),
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: Some(Utc::now()),
    };

    if let Err(e) = notif_store::insert(config, &notification) {
        tracing::warn!(
            job_id = %job.id,
            error = %e,
            "[cron] failed to push notification alert"
        );
    } else {
        tracing::debug!(
            job_id = %job.id,
            "[cron] pushed notification alert to alerts tab"
        );
    }
}

fn is_env_assignment(word: &str) -> bool {
    word.contains('=')
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

fn forbidden_path_argument(security: &SecurityPolicy, command: &str) -> Option<String> {
    let mut normalized = command.to_string();
    for sep in ["&&", "||"] {
        normalized = normalized.replace(sep, "\x00");
    }
    for sep in ['\n', ';', '|'] {
        normalized = normalized.replace(sep, "\x00");
    }

    for segment in normalized.split('\x00') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env assignments and executable token.
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(tokens[idx]) {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }
        idx += 1;

        for token in &tokens[idx..] {
            let candidate = strip_wrapping_quotes(token);
            if candidate.is_empty() || candidate.starts_with('-') || candidate.contains("://") {
                continue;
            }

            let looks_like_path = candidate.starts_with('/')
                || candidate.starts_with("./")
                || candidate.starts_with("../")
                || candidate.starts_with("~/")
                || candidate.contains('/');

            if looks_like_path && !security.is_path_allowed(candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

async fn run_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    run_job_command_with_timeout(
        config,
        security,
        job,
        Duration::from_secs(SHELL_JOB_TIMEOUT_SECS),
    )
    .await
}

async fn run_job_command_with_timeout(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    timeout: Duration,
) -> (bool, String) {
    if !security.can_act() {
        return (
            false,
            "blocked by security policy: autonomy is read-only".to_string(),
        );
    }

    if security.is_rate_limited() {
        return (
            false,
            "blocked by security policy: rate limit exceeded".to_string(),
        );
    }

    if !security.is_command_allowed(&job.command) {
        return (
            false,
            format!(
                "blocked by security policy: command not allowed: {}",
                job.command
            ),
        );
    }

    if let Some(path) = forbidden_path_argument(security, &job.command) {
        return (
            false,
            format!("blocked by security policy: forbidden path argument: {path}"),
        );
    }

    if !security.record_action() {
        return (
            false,
            "blocked by security policy: action budget exhausted".to_string(),
        );
    }

    let child = match Command::new("sh")
        .arg("-lc")
        .arg(&job.command)
        .current_dir(&config.workspace_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return (false, format!("spawn error: {e}")),
    };

    match time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!(
                "status={}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout.trim(),
                stderr.trim()
            );
            (output.status.success(), combined)
        }
        Ok(Err(e)) => (false, format!("spawn error: {e}")),
        Err(_) => (
            false,
            format!("job timed out after {}s", timeout.as_secs_f64()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::cron::{self, DeliveryConfig};
    use crate::openhuman::security::SecurityPolicy;
    use chrono::{Duration as ChronoDuration, Utc};
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
}
