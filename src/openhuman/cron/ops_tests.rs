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
    assert_eq!(parse_delay("5s").unwrap(), chrono::Duration::seconds(5));
    assert_eq!(parse_delay("10m").unwrap(), chrono::Duration::minutes(10));
    assert_eq!(parse_delay("2h").unwrap(), chrono::Duration::hours(2));
    assert_eq!(parse_delay("3d").unwrap(), chrono::Duration::days(3));
}

#[test]
fn parse_delay_defaults_to_minutes_when_no_unit() {
    assert_eq!(parse_delay("15").unwrap(), chrono::Duration::minutes(15));
}

#[test]
fn parse_delay_trims_whitespace() {
    assert_eq!(parse_delay("  7m  ").unwrap(), chrono::Duration::minutes(7));
}

#[test]
fn parse_delay_rejects_empty_input() {
    let err = parse_delay("").unwrap_err();
    assert!(err.to_string().contains("delay must not be empty"));
    let err = parse_delay("   ").unwrap_err();
    assert!(err.to_string().contains("delay must not be empty"));
}

#[test]
fn parse_delay_rejects_unsupported_unit() {
    let err = parse_delay("5x").unwrap_err();
    assert!(err.to_string().contains("unsupported delay unit"));
    // Multi-char unit not matched in the parse branch either.
    let err = parse_delay("5wk").unwrap_err();
    assert!(err.to_string().contains("unsupported delay unit"));
}

#[test]
fn parse_delay_rejects_non_numeric_prefix() {
    // No ascii-digit prefix at all → empty num, parse() fails.
    assert!(parse_delay("abc").is_err());
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
