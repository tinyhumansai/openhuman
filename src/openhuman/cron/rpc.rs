//! JSON-RPC / CLI controller surface for scheduled jobs.

use chrono::Utc;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, CronJob, CronJobPatch, CronRun};
use crate::openhuman::security::SecurityPolicy;
use crate::rpc::RpcOutcome;

pub async fn cron_list(config: &Config) -> Result<RpcOutcome<Vec<CronJob>>, String> {
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }
    let jobs = cron::list_jobs(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(jobs, "cron jobs listed"))
}

pub async fn cron_update(
    config: &Config,
    job_id: &str,
    patch: CronJobPatch,
) -> Result<RpcOutcome<CronJob>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }

    if let Some(command) = &patch.command {
        let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
        if !security.is_command_allowed(command) {
            return Err(format!("Command blocked by security policy: {command}"));
        }
    }

    let updated = cron::update_job(config, job_id.trim(), patch).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        updated,
        vec![format!("cron job updated: {}", job_id.trim())],
    ))
}

pub async fn cron_remove(
    config: &Config,
    job_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }

    cron::remove_job(config, job_id.trim()).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        json!({ "job_id": job_id.trim(), "removed": true }),
        vec![format!("cron job removed: {}", job_id.trim())],
    ))
}

pub async fn cron_run(
    config: &Config,
    job_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }

    let job = cron::get_job(config, job_id.trim()).map_err(|e| e.to_string())?;
    let started_at = Utc::now();
    let (success, output) = cron::scheduler::execute_job_now(config, &job).await;
    let finished_at = Utc::now();
    let duration_ms = (finished_at - started_at).num_milliseconds();
    let status = if success { "ok" } else { "error" };

    let _ = cron::record_run(
        config,
        &job.id,
        started_at,
        finished_at,
        status,
        Some(&output),
        duration_ms,
    );
    let _ = cron::record_last_run(config, &job.id, finished_at, success, &output);

    Ok(RpcOutcome::new(
        json!({
            "job_id": job.id,
            "status": status,
            "duration_ms": duration_ms,
            "output": output
        }),
        vec![format!("cron job run: {}", job_id.trim())],
    ))
}

pub async fn cron_runs(
    config: &Config,
    job_id: &str,
    limit: Option<usize>,
) -> Result<RpcOutcome<Vec<CronRun>>, String> {
    if job_id.trim().is_empty() {
        return Err("Missing 'job_id' parameter".to_string());
    }
    if !config.cron.enabled {
        return Err("cron is disabled by config (cron.enabled=false)".to_string());
    }

    let limit = limit.unwrap_or(20).max(1);
    let runs = cron::list_runs(config, job_id.trim(), limit).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        runs,
        vec![format!("cron run history loaded: {}", job_id.trim())],
    ))
}
