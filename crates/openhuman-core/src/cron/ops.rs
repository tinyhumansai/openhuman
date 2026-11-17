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

    let job = load_job_for_run(config, job_id).map_err(|e| e.to_string())?;
    cron::limits::check_run(config, &job).map_err(|e| e.to_string())?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let reply = cron::scheduler::run_job_now(
        ctx,
        config,
        job,
        run_id,
        false,
        tinyflows_bus::schedule::ManualDelivery::Requested,
    )
    .await
    .map_err(|error| error.to_string())?;
    match reply {
        tinyflows_bus::schedule::RunJobReply::Accepted { .. } => {}
        tinyflows_bus::schedule::RunJobReply::Refused { reason } => {
            return Err(format!("cron job '{job_id}' was not queued: {reason:?}"));
        }
        tinyflows_bus::schedule::RunJobReply::Completed { .. } => {
            return Err("cron module completed a detached run before acknowledgment".to_string());
        }
    }

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
    let run_id = uuid::Uuid::new_v4().to_string();
    match cron::scheduler::run_job_now(
        ctx,
        config,
        job.clone(),
        run_id,
        true,
        tinyflows_bus::schedule::ManualDelivery::Requested,
    )
    .await
    .map_err(|error| error.to_string())?
    {
        tinyflows_bus::schedule::RunJobReply::Completed {
            success, attempt, ..
        } => Ok((success, attempt.output)),
        tinyflows_bus::schedule::RunJobReply::Refused { reason } => Err(format!(
            "cron job '{}' was not admitted: {reason:?}",
            job.id
        )),
        tinyflows_bus::schedule::RunJobReply::Accepted { .. } => {
            Err("cron module accepted a synchronous run without completing it".to_string())
        }
    }
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

#[cfg(all(test, feature = "channels"))]
#[path = "ops_delivery_tests.rs"]
mod delivery_tests;
