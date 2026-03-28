use chrono::Utc;
use serde_json::json;

use crate::core_server::helpers::{load_openhuman_config, parse_params};
use crate::core_server::types::{CronJobIdParams, CronRunsParams, CronUpdateParams, InvocationResult};
use crate::openhuman::cron;
use crate::openhuman::security::SecurityPolicy;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.cron_list" => Some(
            async move {
                let config = load_openhuman_config().await?;
                if !config.cron.enabled {
                    return Err("cron is disabled by config (cron.enabled=false)".to_string());
                }
                let jobs = cron::list_jobs(&config).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(jobs, vec!["cron jobs listed".to_string()])
            }
            .await,
        ),

        "openhuman.cron_update" => Some(
            async move {
                let payload: CronUpdateParams = parse_params(params)?;
                if payload.job_id.trim().is_empty() {
                    return Err("Missing 'job_id' parameter".to_string());
                }

                let config = load_openhuman_config().await?;
                if !config.cron.enabled {
                    return Err("cron is disabled by config (cron.enabled=false)".to_string());
                }

                if let Some(command) = &payload.patch.command {
                    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
                    if !security.is_command_allowed(command) {
                        return Err(format!("Command blocked by security policy: {command}"));
                    }
                }

                let updated = cron::update_job(&config, payload.job_id.trim(), payload.patch)
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    updated,
                    vec![format!("cron job updated: {}", payload.job_id.trim())],
                )
            }
            .await,
        ),

        "openhuman.cron_remove" => Some(
            async move {
                let payload: CronJobIdParams = parse_params(params)?;
                if payload.job_id.trim().is_empty() {
                    return Err("Missing 'job_id' parameter".to_string());
                }

                let config = load_openhuman_config().await?;
                if !config.cron.enabled {
                    return Err("cron is disabled by config (cron.enabled=false)".to_string());
                }

                cron::remove_job(&config, payload.job_id.trim()).map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    json!({ "job_id": payload.job_id.trim(), "removed": true }),
                    vec![format!("cron job removed: {}", payload.job_id.trim())],
                )
            }
            .await,
        ),

        "openhuman.cron_run" => Some(
            async move {
                let payload: CronJobIdParams = parse_params(params)?;
                if payload.job_id.trim().is_empty() {
                    return Err("Missing 'job_id' parameter".to_string());
                }

                let config = load_openhuman_config().await?;
                if !config.cron.enabled {
                    return Err("cron is disabled by config (cron.enabled=false)".to_string());
                }

                let job = cron::get_job(&config, payload.job_id.trim()).map_err(|e| e.to_string())?;
                let started_at = Utc::now();
                let (success, output) = cron::scheduler::execute_job_now(&config, &job).await;
                let finished_at = Utc::now();
                let duration_ms = (finished_at - started_at).num_milliseconds();
                let status = if success { "ok" } else { "error" };

                let _ = cron::record_run(
                    &config,
                    &job.id,
                    started_at,
                    finished_at,
                    status,
                    Some(&output),
                    duration_ms,
                );
                let _ = cron::record_last_run(&config, &job.id, finished_at, success, &output);

                InvocationResult::with_logs(
                    json!({
                        "job_id": job.id,
                        "status": status,
                        "duration_ms": duration_ms,
                        "output": output
                    }),
                    vec![format!("cron job run: {}", payload.job_id.trim())],
                )
            }
            .await,
        ),

        "openhuman.cron_runs" => Some(
            async move {
                let payload: CronRunsParams = parse_params(params)?;
                if payload.job_id.trim().is_empty() {
                    return Err("Missing 'job_id' parameter".to_string());
                }

                let config = load_openhuman_config().await?;
                if !config.cron.enabled {
                    return Err("cron is disabled by config (cron.enabled=false)".to_string());
                }

                let limit = payload.limit.unwrap_or(20).max(1);
                let runs = cron::list_runs(&config, payload.job_id.trim(), limit)
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    runs,
                    vec![format!(
                        "cron run history loaded: {}",
                        payload.job_id.trim()
                    )],
                )
            }
            .await,
        ),

        _ => None,
    }
}
