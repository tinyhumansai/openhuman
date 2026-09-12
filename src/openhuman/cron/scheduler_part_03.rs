async fn persist_job_result(
    config: &Config,
    job: &CronJob,
    mut success: bool,
    output: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> bool {
    let duration_ms = (finished_at - started_at).num_milliseconds();

    if let Err(e) = deliver_if_configured(config, job, output, success).await {
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

    // A fixed-instant (`Schedule::At`) job is inherently one-shot: its `at` is in
    // the past the moment it runs, so `reschedule_after_run` (which writes
    // next_run = at for an `At` schedule) leaves next_run <= now and the job is
    // re-selected by `due_jobs` on every poll, re-executing forever. Terminate
    // every `At` job after a single run, regardless of `delete_after_run`. Only an
    // auto-delete job that succeeded is removed; everything else is kept disabled
    // so its run history stays inspectable. (Inside this `At` branch
    // `is_one_shot_auto_delete` reduces to `job.delete_after_run`.)
    if matches!(job.schedule, Schedule::At { .. }) {
        if is_one_shot_auto_delete(job) && success {
            if let Err(e) = remove_job(config, &job.id) {
                tracing::warn!("Failed to remove one-shot cron job after success: {e}");
            }
        } else {
            let _ = record_last_run(config, &job.id, finished_at, success, output);
            if let Err(e) = update_job(
                config,
                &job.id,
                CronJobPatch {
                    enabled: Some(false),
                    ..CronJobPatch::default()
                },
            ) {
                tracing::warn!("Failed to disable one-shot cron job: {e}");
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

/// Why an agent job counts as scheduled more often than every five minutes,
/// if it does. `None` for shell and flow jobs, for one-shot `At` schedules,
/// for anything at or above [`MIN_AGENT_JOB_INTERVAL`], and for an expression
/// that cannot be read (that is not evidence of anything).
///
/// Returns the verdict rather than only logging it, because the verdict is
/// the part worth testing: the `Schedule::Cron` arm used to measure the gap
/// between the next run after now and the next run after now plus one
/// second — the same instant unless a run fell inside that second — so every
/// cron-scheduled agent job warned, and no test could see it because a
/// function that only warns has nothing to assert. The gap is now the
/// shortest one between consecutive runs (`runs_closer_than`), so an
/// irregular expression is judged by its tightest pair and not by whichever
/// pair happens to follow the instant of the check.
fn agent_job_too_frequent(job: &CronJob) -> Option<TooFrequent> {
    if !matches!(job.job_type, JobType::Agent) {
        return None;
    }
    runs_closer_than(&job.schedule, Utc::now(), MIN_AGENT_JOB_INTERVAL)
}

/// New agent jobs cannot be created below the floor (`validate_agent_schedule`
/// rejects them), so a job that trips this predates the rule. It keeps
/// running — silently skipping runs would turn its schedule into a lie — and
/// this line is the operator's signal to edit it.
fn warn_if_high_frequency_agent_job(job: &CronJob) {
    if let Some(too_frequent) = agent_job_too_frequent(job) {
        tracing::warn!(
            "Cron agent job '{}' is scheduled more frequently than every 5 minutes: it \
             {too_frequent}. It was created before that floor was enforced; edit its \
             schedule to silence this.",
            job.id
        );
    }
}

/// True when an agent job produced no meaningful text — blank output or the
/// [`EMPTY_AGENT_OUTPUT`] placeholder. Such runs are never injected into chat.
fn cron_output_is_empty(output: &str) -> bool {
    output.trim().is_empty() || output == EMPTY_AGENT_OUTPUT
}

/// Whether a cron job's output should be injected into the user's chat thread.
/// Skips failed runs and empty/placeholder output; failures still surface in
/// the alerts tab and run history (handled separately by the caller).
fn should_deliver_cron_output_to_chat(success: bool, output: &str) -> bool {
    success && !cron_output_is_empty(output)
}

/// Whether a completed cron run should surface in the alerts tab
/// (`/notifications`). Failures stay visible even when they produce no output;
/// only successful-but-empty runs are dropped entirely.
fn cron_result_should_alert(success: bool, output: &str) -> bool {
    !success || !cron_output_is_empty(output)
}

async fn deliver_if_configured(
    config: &Config,
    job: &CronJob,
    output: &str,
    success: bool,
) -> Result<()> {
    let delivery: &DeliveryConfig = &job.delivery;

    // Don't post failed or empty cron runs into the user's chat: a failed turn
    // (e.g. a transient network error) would otherwise deliver a canned
    // "Something went wrong" message into the conversation with no user
    // message behind it. Failures still reach the alerts tab (`push_cron_alert`)
    // and the run-history / health signals, which are recorded elsewhere.
    let is_empty = cron_output_is_empty(output);
    let deliver_to_chat = should_deliver_cron_output_to_chat(success, output);
    if !deliver_to_chat {
        tracing::debug!(
            job_id = %job.id,
            success,
            is_empty,
            "[cron] skipping chat delivery for failed/empty cron run"
        );
    }

    // A failed run must stay visible in /notifications regardless of delivery
    // mode — a no-delivery agent job that halts on a permanent config/billing
    // state (e.g. a keyless provider, TAURI-RUST-HCK) would otherwise fail
    // silently. A *successful* non-empty run only alerts in the delivering
    // modes (proactive/announce); a `none`-mode success stays silent (its
    // output lives in last_output only — the cron contract), so we don't spam
    // explicitly-silent background jobs with an unread alert every interval
    // (Codex #4166).
    let mode = delivery.mode.trim().to_ascii_lowercase();
    let delivers = matches!(mode.as_str(), "proactive" | "announce");
    let alert_to_notifications =
        cron_result_should_alert(success, output) && (!success || delivers);
    let alert_body = if is_empty {
        "Scheduled job failed without output."
    } else {
        output
    };

    match mode.as_str() {
        // Proactive delivery — the channels module decides where to send.
        // Used by morning briefings, welcome messages, and other
        // user-facing proactive agents.
        "proactive" => {
            if deliver_to_chat {
                let source = format!("cron:{}", job.id);
                tracing::debug!(
                    job_id = %job.id,
                    source = %source,
                    "[cron] publishing ProactiveMessageRequested event"
                );
                BUS.publish(DomainEvent::ProactiveMessageRequested {
                    source,
                    message: output.to_string(),
                    job_name: job.name.clone(),
                });
            }
        }

        // Announce delivery — the cron job specifies the exact channel
        // and target. Used for explicit channel-targeted output.
        "announce" if deliver_to_chat => {
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
            BUS.publish(DomainEvent::CronDeliveryRequested {
                job_id: job.id.clone(),
                channel: channel.to_string(),
                target: target.to_string(),
                output: output.to_string(),
            });
        }

        // No delivery configured — output is stored in last_output only.
        // The failure still reaches the alerts tab via the hoisted
        // `push_cron_alert` below.
        _ => {}
    }

    // Surface in the alerts tab (/notifications) for any result that isn't a
    // successful-but-empty run — INDEPENDENT of delivery mode. A failed cron
    // job must stay visible to the user even when it has no chat delivery
    // configured (the common case: a keyless agent job failing "API key not
    // set", TAURI-RUST-HCK). Previously this fired only inside the proactive /
    // announce arms, so no-delivery jobs failed silently in /notifications.
    if alert_to_notifications {
        push_cron_alert(config, job, alert_body);
    }

    Ok(())
}

/// Insert a notification into the alerts tab for a completed cron job.
fn push_cron_alert(config: &Config, job: &CronJob, output: &str) {
    use crate::openhuman::desktop::notifications::store as notif_store;
    use crate::openhuman::desktop::notifications::types::{
        IntegrationNotification, NotificationStatus,
    };

    let name = job.name.as_deref().unwrap_or("Cron job");
    let body = cron_alert_body(job, output);

    let notification = IntegrationNotification {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "cron".to_string(),
        account_id: Some(job.id.clone()),
        title: name.to_string(),
        body,
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

    match notif_store::insert_if_not_recent(config, &notification) {
        Ok(true) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] pushed notification alert to alerts tab"
            );
        }
        Ok(false) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] skipped duplicate notification alert"
            );
        }
        Err(e) => {
            tracing::warn!(
                job_id = %job.id,
                error = %e,
                "[cron] failed to push notification alert"
            );
        }
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

            if looks_like_path && !security.is_path_string_allowed(candidate) {
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
        .current_dir(&config.action_dir)
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
