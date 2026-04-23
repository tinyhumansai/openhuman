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
        let (existing_expr, existing_tz) = match existing.schedule {
            Schedule::Cron {
                expr,
                tz: existing_tz,
            } => (expr, existing_tz),
            _ => anyhow::bail!("Cannot update expression/tz on a non-cron schedule"),
        };
        Some(Schedule::Cron {
            expr: expression.unwrap_or(existing_expr),
            tz: tz.or(existing_tz),
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    fn make_job(config: &Config, expr: &str, tz: Option<&str>, cmd: &str) -> CronJob {
        add_shell_job(
            config,
            None,
            Schedule::Cron {
                expr: expr.into(),
                tz: tz.map(Into::into),
            },
            cmd,
        )
        .unwrap()
    }

    fn run_update(
        config: &Config,
        id: &str,
        expression: Option<&str>,
        tz: Option<&str>,
        command: Option<&str>,
        name: Option<&str>,
    ) -> Result<()> {
        update_cron_job(
            config,
            id,
            expression.map(Into::into),
            tz.map(Into::into),
            command.map(Into::into),
            name.map(Into::into),
        )
        .map(|_| ())
    }

    #[test]
    fn update_changes_command_via_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo original");

        run_update(&config, &job.id, None, None, Some("echo updated"), None).unwrap();

        let updated = get_job(&config, &job.id).unwrap();
        assert_eq!(updated.command, "echo updated");
        assert_eq!(updated.id, job.id);
    }

    #[test]
    fn update_changes_expression_via_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");

        run_update(&config, &job.id, Some("0 9 * * *"), None, None, None).unwrap();

        let updated = get_job(&config, &job.id).unwrap();
        assert_eq!(updated.expression, "0 9 * * *");
    }

    #[test]
    fn update_changes_name_via_handler() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");

        run_update(&config, &job.id, None, None, None, Some("new-name")).unwrap();

        let updated = get_job(&config, &job.id).unwrap();
        assert_eq!(updated.name.as_deref(), Some("new-name"));
    }

    #[test]
    fn update_tz_alone_sets_timezone() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");

        run_update(
            &config,
            &job.id,
            None,
            Some("America/Los_Angeles"),
            None,
            None,
        )
        .unwrap();

        let updated = get_job(&config, &job.id).unwrap();
        assert_eq!(
            updated.schedule,
            Schedule::Cron {
                expr: "*/5 * * * *".into(),
                tz: Some("America/Los_Angeles".into()),
            }
        );
    }

    #[test]
    fn update_expr_alone_preserves_timezone() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", Some("UTC"), "echo test");

        run_update(&config, &job.id, Some("0 10 * * *"), None, None, None).unwrap();

        let updated = get_job(&config, &job.id).unwrap();
        assert_eq!(
            updated.schedule,
            Schedule::Cron {
                expr: "0 10 * * *".into(),
                tz: Some("UTC".into()),
            }
        );
    }

    #[test]
    fn update_fails_when_no_fields_provided() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");

        let err = run_update(&config, &job.id, None, None, None, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("At least one of --expression, --tz, --command, or --name"));
    }

    #[test]
    fn update_rejects_expression_for_non_cron_schedule() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let at = chrono::Utc::now() + chrono::Duration::minutes(5);
        let job = add_shell_job(&config, None, Schedule::At { at }, "echo test").unwrap();

        let err = run_update(&config, &job.id, Some("0 9 * * *"), None, None, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("Cannot update expression/tz on a non-cron schedule"));
    }

    // ── parse_delay ─────────────────────────────────────────────────

    #[test]
    fn parse_delay_accepts_seconds_minutes_hours_days() {
        assert_eq!(parse_human_delay("5s").unwrap(), chrono::Duration::seconds(5));
        assert_eq!(parse_human_delay("10m").unwrap(), chrono::Duration::minutes(10));
        assert_eq!(parse_human_delay("2h").unwrap(), chrono::Duration::hours(2));
        assert_eq!(parse_human_delay("3d").unwrap(), chrono::Duration::days(3));
    }

    #[test]
    fn parse_delay_defaults_to_minutes_when_no_unit() {
        assert_eq!(parse_human_delay("15").unwrap(), chrono::Duration::minutes(15));
    }

    #[test]
    fn parse_delay_trims_whitespace() {
        assert_eq!(parse_human_delay("  7m  ").unwrap(), chrono::Duration::minutes(7));
    }

    #[test]
    fn parse_delay_rejects_empty_input() {
        let err = parse_human_delay("").unwrap_err();
        assert!(err.to_string().contains("delay must not be empty"));
        let err = parse_human_delay("   ").unwrap_err();
        assert!(err.to_string().contains("delay must not be empty"));
    }

    #[test]
    fn parse_delay_rejects_unsupported_unit() {
        let err = parse_human_delay("5x").unwrap_err();
        assert!(err.to_string().contains("unsupported delay unit"));
        // Multi-char unit not matched in the parse branch either.
        let err = parse_human_delay("5wk").unwrap_err();
        assert!(err.to_string().contains("unsupported delay unit"));
    }

    #[test]
    fn parse_delay_rejects_non_numeric_prefix() {
        // No ascii-digit prefix at all → empty num, parse() fails.
        assert!(parse_human_delay("abc").is_err());
    }

    // ── add_once ────────────────────────────────────────────────────

    #[test]
    fn add_once_creates_future_at_schedule() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let before = chrono::Utc::now();
        let job = add_once(&config, "5m", "echo hello").unwrap();
        match job.schedule {
            Schedule::At { at } => {
                let min = before + chrono::Duration::minutes(5) - chrono::Duration::seconds(2);
                let max = before + chrono::Duration::minutes(5) + chrono::Duration::seconds(5);
                assert!(at > min && at < max, "scheduled 'at' should land ~5m out");
            }
            other => panic!("expected At schedule, got {other:?}"),
        }
        assert_eq!(job.command, "echo hello");
    }

    #[test]
    fn add_once_propagates_parse_delay_errors() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        assert!(add_once(&config, "", "cmd").is_err());
        assert!(add_once(&config, "5x", "cmd").is_err());
    }

    // ── add_once_at ─────────────────────────────────────────────────

    #[test]
    fn add_once_at_stores_exact_timestamp() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let when = chrono::Utc::now() + chrono::Duration::hours(1);
        let job = add_once_at(&config, when, "echo hi").unwrap();
        match job.schedule {
            Schedule::At { at } => assert_eq!(at, when),
            other => panic!("expected At schedule, got {other:?}"),
        }
    }

    // ── pause_job / resume_job ──────────────────────────────────────

    #[test]
    fn pause_and_resume_toggle_enabled_flag() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");
        assert!(job.enabled);

        let paused = pause_job(&config, &job.id).unwrap();
        assert!(!paused.enabled);

        let resumed = resume_job(&config, &job.id).unwrap();
        assert!(resumed.enabled);
    }

    // ── cron_list / cron_update / cron_remove / cron_runs ───────────

    fn disabled_cron_config(tmp: &TempDir) -> Config {
        let mut config = test_config(tmp);
        config.cron.enabled = false;
        config
    }

    #[tokio::test]
    async fn cron_list_errors_when_cron_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = disabled_cron_config(&tmp);
        let err = cron_list(&config).await.unwrap_err();
        assert!(err.contains("cron is disabled"));
    }

    #[tokio::test]
    async fn cron_list_returns_jobs_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");
        let out = cron_list(&config).await.unwrap();
        assert!(out.value.iter().any(|j| j.id == job.id));
        assert!(out.logs.iter().any(|l| l.contains("cron jobs listed")));
    }

    #[tokio::test]
    async fn cron_update_rejects_empty_job_id() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let err = cron_update(&config, "   ", CronJobPatch::default())
            .await
            .unwrap_err();
        assert!(err.contains("Missing 'job_id'"));
    }

    #[tokio::test]
    async fn cron_update_errors_when_cron_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = disabled_cron_config(&tmp);
        let err = cron_update(&config, "some-id", CronJobPatch::default())
            .await
            .unwrap_err();
        assert!(err.contains("cron is disabled"));
    }

    #[tokio::test]
    async fn cron_update_mutates_existing_job() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");
        let patch = CronJobPatch {
            name: Some("renamed".to_string()),
            ..CronJobPatch::default()
        };
        let out = cron_update(&config, &job.id, patch).await.unwrap();
        assert_eq!(out.value.name.as_deref(), Some("renamed"));
        assert!(out.logs.iter().any(|l| l.contains("cron job updated")));
    }

    #[tokio::test]
    async fn cron_remove_rejects_empty_job_id() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let err = cron_remove(&config, "").await.unwrap_err();
        assert!(err.contains("Missing 'job_id'"));
    }

    #[tokio::test]
    async fn cron_remove_errors_when_cron_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = disabled_cron_config(&tmp);
        let err = cron_remove(&config, "abc").await.unwrap_err();
        assert!(err.contains("cron is disabled"));
    }

    #[tokio::test]
    async fn cron_remove_returns_removed_true_on_success() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");
        let out = cron_remove(&config, &job.id).await.unwrap();
        assert_eq!(out.value["job_id"], json!(job.id));
        assert_eq!(out.value["removed"], json!(true));
    }

    #[tokio::test]
    async fn cron_runs_rejects_empty_job_id() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let err = cron_runs(&config, "", None).await.unwrap_err();
        assert!(err.contains("Missing 'job_id'"));
    }

    #[tokio::test]
    async fn cron_runs_errors_when_cron_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = disabled_cron_config(&tmp);
        let err = cron_runs(&config, "abc", Some(5)).await.unwrap_err();
        assert!(err.contains("cron is disabled"));
    }

    #[tokio::test]
    async fn cron_runs_returns_empty_history_for_new_job() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let job = make_job(&config, "*/5 * * * *", None, "echo test");
        let out = cron_runs(&config, &job.id, Some(10)).await.unwrap();
        assert!(out.value.is_empty());
        assert!(out.logs.iter().any(|l| l.contains("cron run history")));
    }
}
