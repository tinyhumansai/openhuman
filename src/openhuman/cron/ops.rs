use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    self, add_shell_job, get_job, update_job, CronJob, CronJobPatch, CronRun, Schedule,
};
use crate::openhuman::security::SecurityPolicy;
use crate::rpc::RpcOutcome;
use anyhow::Result;
use serde_json::json;

pub fn add_once(config: &Config, delay: &str, command: &str) -> Result<CronJob> {
    let duration = parse_human_delay(delay)?;
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
        let existing = get_job(config, id)?;
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
        let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
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

/// Parse a human-friendly delay string (e.g. "5m", "2h", "30s") into a
/// `chrono::Duration`. Defaults to minutes when no unit is given.
pub fn parse_human_delay(input: &str) -> Result<chrono::Duration> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("delay must not be empty");
    }
    let split = input
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(input.len());
    let (num, unit) = input.split_at(split);
    let amount: i64 = num.parse()?;
    let unit = if unit.is_empty() { "m" } else { unit };
    let duration = match unit {
        "s" => chrono::Duration::seconds(amount),
        "m" => chrono::Duration::minutes(amount),
        "h" => chrono::Duration::hours(amount),
        "d" => chrono::Duration::days(amount),
        _ => anyhow::bail!("unsupported delay unit '{unit}', use s/m/h/d"),
    };
    Ok(duration)
}

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
    let started_at = chrono::Utc::now();
    let (success, output) = cron::scheduler::execute_job_now(config, &job).await;
    let finished_at = chrono::Utc::now();
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

    // Deliver via the same path as the scheduler loop so proactive
    // messages and alerts are sent on "Run Now" too.
    cron::scheduler::deliver_job(config, &job, &output).await;

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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
