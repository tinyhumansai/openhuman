use crate::config::Config;
use crate::core::Outcome;
use crate::security::SecurityPolicy;
use anyhow::Result;
use serde_json::json;
use {
    crate::cron, crate::cron::add_shell_job, crate::cron::update_job,
    tinyflows_bus::schedule::types::CronJob, tinyflows_bus::schedule::types::CronJobPatch,
    tinyflows_bus::schedule::types::CronRun, tinyflows_bus::schedule::types::Schedule,
};

/// Claim `job_id` as running. `None` when a Run Now of it already holds it. The
/// claim is released when the guard drops, including on panic or future
/// cancellation, so an aborted task can never lock a job id. A *scheduled* run
/// of the same job is refused by the module's own claim when the run starts.
pub(crate) fn try_acquire_run(job_id: &str) -> Option<crate::cron::scheduler::RunGuard> {
    crate::cron::scheduler::try_acquire_run(job_id)
}

/// Loads `job_id` for a run, failing closed. The job document carries the
/// run policy (retry budget, single-flight), so a store that cannot produce it
/// refuses the run. A missing job reads as such; any other error is reduced to
/// the job id, because backend error text can carry credentials or payloads.
pub(crate) fn load_job_for_run(config: &Config, job_id: &str) -> Result<CronJob> {
    cron::get_job(config, job_id).map_err(|error| {
        if error.to_string() == format!("Cron job '{job_id}' not found") {
            return error;
        }
        tracing::warn!(job_id, "[cron_run] job unreadable; refusing to run");
        anyhow::anyhow!("cron job {job_id} is unavailable; refusing to run it")
    })
}

pub fn add_once(config: &Config, delay: &str, command: &str) -> Result<CronJob> {
    let delay = delay.to_string();
    let duration = crate::cron::client::blocking(config, move |config| async move {
        crate::cron::client::parse_human_delay(&config, &delay).await
    })?;
    let at = chrono::Utc::now() + duration;
    add_once_at(config, at, command)
}

pub fn add_once_at(
    config: &Config,
    at: chrono::DateTime<chrono::Utc>,
    command: &str,
) -> Result<CronJob> {
    let schedule = Schedule::At { at };
    add_shell_job(config, None, schedule, command)
}

pub fn pause_job(config: &Config, id: &str) -> Result<CronJob> {
    update_job(
        config,
        id,
        CronJobPatch {
            enabled: Some(false),
            ..CronJobPatch::default()
        },
    )
}

pub fn resume_job(config: &Config, id: &str) -> Result<CronJob> {
    update_job(
        config,
        id,
        CronJobPatch {
            enabled: Some(true),
            ..CronJobPatch::default()
        },
    )
}

/// Update an existing cron job using the same rules as the legacy CLI, but without CLI wiring.
///
/// `expression` and `tz` are merged with the existing [`Schedule::Cron`] fields; the
/// existing `active_hours` is always preserved as-is.  To set or clear `active_hours`
/// directly, use the RPC path (`cron.update` with a full [`CronJobPatch`]).
#[cfg(test)]
pub fn update_cron_job(
    config: &Config,
    id: &str,
    expression: Option<String>,
    tz: Option<String>,
    command: Option<String>,
    name: Option<String>,
) -> Result<CronJob> {
    if expression.is_none() && tz.is_none() && command.is_none() && name.is_none() {
        anyhow::bail!("At least one of --expression, --tz, --command, or --name must be provided");
    }

    // Merge expression/tz with the existing schedule so that
    // tz alone updates the timezone and expression alone preserves the timezone.
    let schedule = if expression.is_some() || tz.is_some() {
        let existing = cron::get_job(config, id)?;
        let (existing_expr, existing_tz, existing_active) = match existing.schedule {
            Schedule::Cron {
                expr,
                tz: existing_tz,
                active_hours: existing_active,
            } => (expr, existing_tz, existing_active),
            _ => anyhow::bail!("Cannot update expression/tz on a non-cron schedule"),
        };
        Some(Schedule::Cron {
            expr: expression.unwrap_or(existing_expr),
            tz: tz.or(existing_tz),
            active_hours: existing_active,
        })
    } else {
        None
    };

    if let Some(ref cmd) = command {
        let security = SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        );
        if !security.is_command_allowed(cmd) {
            anyhow::bail!("Command blocked by security policy: {cmd}");
        }
    }

    let patch = CronJobPatch {
        schedule,
        command,
        name,
        ..CronJobPatch::default()
    };

    update_job(config, id, patch)
}

pub async fn cron_list(config: &Config) -> Result<Outcome<Vec<CronJob>>, String> {
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    crate::core::runtime::components::require_config(config, "tinyflows")?;
    let jobs = cron::list_jobs(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(jobs, "cron jobs listed"))
}

pub async fn cron_update(
    config: &Config,
    job_id: &str,
    patch: CronJobPatch,
) -> Result<Outcome<CronJob>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    crate::core::runtime::components::require_config(config, "tinyflows")?;

    if let Some(command) = &patch.command {
        let security = SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        );
        crate::cron::scheduler::shell_job::authorize_shell_command(config, &security, command)
            .await?;
    }

    let updated = cron::update_job(config, job_id.trim(), patch).map_err(|e| e.to_string())?;
    Ok(Outcome::new(
        updated,
        vec![format!("cron job updated: {}", job_id.trim())],
    ))
}

pub async fn cron_remove(
    config: &Config,
    job_id: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    crate::core::runtime::components::require_config(config, "tinyflows")?;

    cron::remove_job(config, job_id.trim()).map_err(|e| e.to_string())?;
    Ok(Outcome::new(
        json!({ "job_id": job_id.trim(), "removed": true }),
        vec![format!("cron job removed: {}", job_id.trim())],
    ))
}

pub async fn cron_run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job_id: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    let job_id = job_id.trim();
    if job_id.is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    crate::core::runtime::components::require_config(config, "tinyflows")?;

    // Fail closed before queueing anything: an unreadable job refuses the run.
    cron::limits::check_run(
        config,
        &load_job_for_run(config, job_id).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    // Guard against duplicate concurrent executions for the same job. The
    // claim is taken before the job is loaded and moves into the task.
    let Some(run_guard) = try_acquire_run(job_id) else {
        tracing::debug!(job_id, "[cron_run] rejected duplicate concurrent execution");
        return Err(format!("cron job '{job_id}' is already running"));
    };
    let job = load_job_for_run(config, job_id).map_err(|e| e.to_string())?;

    // Insert a "queued" placeholder run record immediately so the frontend
    // poller can observe the run as soon as the RPC returns — otherwise the
    // run list stays unchanged until execute_job_now finishes.
    let queued_at = chrono::Utc::now();
    let _ = cron::record_run(config, &job.id, queued_at, queued_at, "queued", None, 0);

    let config_owned = config.clone();
    let ctx_owned = ctx.clone();
    let job_id_owned = job_id.to_string();

    tracing::debug!(job_id, "[cron_run] enqueuing background execution");

    // Spawn the execution as a background task so the RPC handler returns
    // immediately instead of blocking for the full job duration.
    tokio::spawn(async move {
        // The claim is released even on panic or future cancellation.
        let _guard = run_guard;

        tracing::debug!(job_id = %job_id_owned, "[cron_run] background task started");
        run_and_record(&ctx_owned, &config_owned, &job).await;
    });

    Ok(Outcome::new(
        json!({
            "job_id": job_id,
            "status": "queued",
        }),
        vec![format!("cron job enqueued: {}", job_id)],
    ))
}

/// Run job `job_id` now and wait for it: the same execution, retry budget,
/// delivery and run record as a scheduled run. Refused while a run of the job
/// (scheduled or another Run Now) is already active.
pub async fn run_job_now(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job_id: &str,
) -> Result<(bool, String), String> {
    let job = load_job_for_run(config, job_id.trim()).map_err(|e| e.to_string())?;
    cron::limits::check_run(config, &job).map_err(|e| e.to_string())?;
    let Some(_guard) = try_acquire_run(&job.id) else {
        tracing::debug!(job_id = %job.id, "[cron_run] job already running; refused");
        return Err(format!("cron job '{}' is already running", job.id));
    };
    Ok(run_and_record(ctx, config, &job).await)
}

/// Execute `job` once (with its retry budget), deliver its output and persist
/// the run and its last-run state. The caller holds the job's run claim.
async fn run_and_record(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
) -> (bool, String) {
    let run_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now();
    let (success, output) = cron::scheduler::execute_job_now(ctx, config, job, &run_id).await;
    if !success && output.ends_with("is already running") {
        // A scheduled run holds the job (the module's claim): nothing ran, so
        // there is nothing to deliver or record, and the "queued" placeholder
        // goes with the refused run.
        tracing::debug!(job_id = %job.id, "[cron_run] refused: a scheduled run holds the job");
        let _ = cron::delete_queued_runs(config, &job.id);
        return (false, output);
    }
    let finished_at = chrono::Utc::now();
    let duration_ms = (finished_at - started_at).num_milliseconds();
    let status = if success { "ok" } else { "error" };
    tracing::debug!(job_id = %job.id, status, duration_ms, "[cron_run] run finished");
    // Remove a "queued" placeholder before inserting the real result so no
    // orphaned rows are left in the run history.
    let _ = cron::delete_queued_runs(config, &job.id);
    // Deliver via the same path as the scheduler loop so proactive messages,
    // origin replies and alerts are sent on "Run Now" too, then record the run
    // with how the delivery went.
    let delivery_status = cron::scheduler::deliver_job(ctx, config, job, &run_id, &output).await;
    let _ = cron::record_run_with_delivery(
        config,
        &job.id,
        started_at,
        finished_at,
        status,
        Some(&output),
        duration_ms,
        Some(delivery_status),
    );
    let _ = cron::record_last_run(config, &job.id, finished_at, success, &output);
    (success, output)
}

pub async fn cron_runs(
    config: &Config,
    job_id: &str,
    limit: Option<usize>,
) -> Result<Outcome<Vec<CronRun>>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    crate::core::runtime::components::require_config(config, "tinyflows")?;

    let limit = limit.unwrap_or(20).max(1);
    let runs = cron::list_runs(config, job_id.trim(), limit).map_err(|e| e.to_string())?;
    Ok(Outcome::new(
        runs,
        vec![format!("cron run history loaded: {}", job_id.trim())],
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
