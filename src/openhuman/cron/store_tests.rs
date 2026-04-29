use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::cron::ActiveHours;
use chrono::Duration as ChronoDuration;
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

#[test]
fn add_job_accepts_five_field_expression() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    assert_eq!(job.expression, "*/5 * * * *");
    assert_eq!(job.command, "echo ok");
    assert!(matches!(job.schedule, Schedule::Cron { .. }));
}

#[test]
fn add_shell_job_persists_active_hours_schedule() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let active_hours = ActiveHours {
        start: "09:00".into(),
        end: "17:00".into(),
    };

    let job = add_shell_job(
        &config,
        Some("business-hours".into()),
        Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(active_hours.clone()),
        },
        "echo ok",
    )
    .unwrap();

    let stored = get_job(&config, &job.id).unwrap();
    assert_eq!(stored.expression, "0 9 * * *");
    assert_eq!(
        stored.schedule,
        Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(active_hours),
        }
    );
}

#[test]
fn add_list_remove_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/10 * * * *", "echo roundtrip").unwrap();
    let listed = list_jobs(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, job.id);

    remove_job(&config, &job.id).unwrap();
    assert!(list_jobs(&config).unwrap().is_empty());
}

#[test]
fn due_jobs_filters_by_timestamp_and_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "* * * * *", "echo due").unwrap();

    let due_now = due_jobs(&config, Utc::now()).unwrap();
    assert!(due_now.is_empty(), "new job should not be due immediately");

    let far_future = Utc::now() + ChronoDuration::days(365);
    let due_future = due_jobs(&config, far_future).unwrap();
    assert_eq!(due_future.len(), 1, "job should be due in far future");

    let _ = update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(false),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    let due_after_disable = due_jobs(&config, far_future).unwrap();
    assert!(due_after_disable.is_empty());
}

#[test]
fn due_jobs_respects_scheduler_max_tasks_limit() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.scheduler.max_tasks = 2;

    let _ = add_job(&config, "* * * * *", "echo due-1").unwrap();
    let _ = add_job(&config, "* * * * *", "echo due-2").unwrap();
    let _ = add_job(&config, "* * * * *", "echo due-3").unwrap();

    let far_future = Utc::now() + ChronoDuration::days(365);
    let due = due_jobs(&config, far_future).unwrap();
    assert_eq!(due.len(), 2);
}

#[test]
fn reschedule_after_run_persists_last_status_and_last_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/15 * * * *", "echo run").unwrap();
    reschedule_after_run(&config, &job, false, "failed output").unwrap();

    let listed = list_jobs(&config).unwrap();
    let stored = listed.iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(stored.last_status.as_deref(), Some("error"));
    assert!(stored.last_run.is_some());
    assert_eq!(stored.last_output.as_deref(), Some("failed output"));
}

#[test]
fn migration_falls_back_to_legacy_expression() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO cron_jobs (id, expression, command, created_at, next_run)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "legacy-id",
                "*/5 * * * *",
                "echo legacy",
                Utc::now().to_rfc3339(),
                (Utc::now() + ChronoDuration::minutes(5)).to_rfc3339(),
            ],
        )?;
        conn.execute(
            "UPDATE cron_jobs SET schedule = NULL WHERE id = 'legacy-id'",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let job = get_job(&config, "legacy-id").unwrap();
    assert!(matches!(job.schedule, Schedule::Cron { .. }));
}

#[test]
fn record_and_prune_runs() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.cron.max_run_history = 2;
    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    let base = Utc::now();

    for idx in 0..3 {
        let start = base + ChronoDuration::seconds(idx);
        let end = start + ChronoDuration::milliseconds(100);
        record_run(&config, &job.id, start, end, "ok", Some("done"), 100).unwrap();
    }

    let runs = list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 2);
}

#[test]
fn remove_job_cascades_run_history() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    let start = Utc::now();
    record_run(
        &config,
        &job.id,
        start,
        start + ChronoDuration::milliseconds(5),
        "ok",
        Some("ok"),
        5,
    )
    .unwrap();

    remove_job(&config, &job.id).unwrap();
    let runs = list_runs(&config, &job.id, 10).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn record_run_truncates_large_output() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo trunc").unwrap();
    let output = "x".repeat(MAX_CRON_OUTPUT_BYTES + 512);

    record_run(
        &config,
        &job.id,
        Utc::now(),
        Utc::now(),
        "ok",
        Some(&output),
        1,
    )
    .unwrap();

    let runs = list_runs(&config, &job.id, 1).unwrap();
    let stored = runs[0].output.as_deref().unwrap_or_default();
    assert!(stored.ends_with(TRUNCATED_OUTPUT_MARKER));
    assert!(stored.len() <= MAX_CRON_OUTPUT_BYTES);
}

#[test]
fn reschedule_after_run_truncates_last_output() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo trunc").unwrap();
    let output = "y".repeat(MAX_CRON_OUTPUT_BYTES + 1024);

    reschedule_after_run(&config, &job, false, &output).unwrap();

    let stored = get_job(&config, &job.id).unwrap();
    let last_output = stored.last_output.as_deref().unwrap_or_default();
    assert!(last_output.ends_with(TRUNCATED_OUTPUT_MARKER));
    assert!(last_output.len() <= MAX_CRON_OUTPUT_BYTES);
}
