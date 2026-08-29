use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::cron::ActiveHours;
use chrono::Duration as ChronoDuration;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
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

    let before_next_run = job.next_run - ChronoDuration::seconds(1);
    let due_before_next_run = due_jobs(&config, before_next_run).unwrap();
    assert!(
        due_before_next_run.is_empty(),
        "job should not be due before its next_run timestamp"
    );

    let due_at_next_run = due_jobs(&config, job.next_run).unwrap();
    assert_eq!(due_at_next_run.len(), 1, "job should be due at next_run");

    let _ = update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(false),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    let due_after_disable = due_jobs(&config, job.next_run).unwrap();
    assert!(due_after_disable.is_empty());
}

#[test]
fn agent_job_round_trips_profile_id_and_patch_clears_it() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Create an agent job attributed to a profile.
    let job = add_agent_job_with_definition(
        &config,
        Some("attributed".into()),
        Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
            active_hours: None,
        },
        "do the thing",
        SessionTarget::Isolated,
        None,
        None,
        false,
        None,
        true,
        Some("alice".into()),
    )
    .unwrap();
    assert_eq!(job.profile_id.as_deref(), Some("alice"));

    // Reload from disk — the column round-trips.
    let stored = get_job(&config, &job.id).unwrap();
    assert_eq!(stored.profile_id.as_deref(), Some("alice"));

    // Patch to a different profile.
    let repointed = update_job(
        &config,
        &job.id,
        CronJobPatch {
            profile_id: Some(Some("bob".into())),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(repointed.profile_id.as_deref(), Some("bob"));

    // Patch with `Some(None)` clears the attribution; `None` leaves it untouched.
    let cleared = update_job(
        &config,
        &job.id,
        CronJobPatch {
            profile_id: Some(None),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(cleared.profile_id, None);

    let untouched = update_job(
        &config,
        &job.id,
        CronJobPatch {
            name: Some("renamed".into()),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(untouched.profile_id, None);
    assert_eq!(untouched.name.as_deref(), Some("renamed"));
}

#[test]
fn shell_job_has_no_profile_attribution() {
    // Back-compat: shell jobs (and any job created without profile_id) load with
    // profile_id = None.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    assert_eq!(job.profile_id, None);
    assert_eq!(get_job(&config, &job.id).unwrap().profile_id, None);
}

#[test]
fn enabling_stale_disabled_job_refreshes_next_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Daily 7 AM job, then disable it (mimics a seeded opt-in morning briefing).
    let job = add_job(&config, "0 7 * * *", "echo briefing").unwrap();
    update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(false),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    // Force a stale next_run in the past, as if the user onboarded before the
    // job's first scheduled fire and only opted in later (hours or days after).
    let stale = Utc::now() - ChronoDuration::hours(2);
    with_connection(&config, |conn| {
        conn.execute(
            "UPDATE cron_jobs SET next_run = ?1 WHERE id = ?2",
            params![stale.to_rfc3339(), job.id],
        )?;
        Ok(())
    })
    .unwrap();

    // Opt in: disabled -> enabled, with the schedule unchanged.
    let enabled = update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(true),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    assert!(enabled.enabled);
    assert!(
        enabled.next_run > Utc::now(),
        "enabling a job with a stale next_run must refresh it to the future, got {}",
        enabled.next_run
    );
    assert!(
        due_jobs(&config, Utc::now()).unwrap().is_empty(),
        "freshly opted-in job must not fire immediately on enable"
    );
}

#[test]
fn enabling_job_with_future_next_run_preserves_it() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "0 7 * * *", "echo briefing").unwrap();
    update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(false),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    // A future next_run is still valid and must be left untouched on enable.
    let future = Utc::now() + ChronoDuration::hours(3);
    with_connection(&config, |conn| {
        conn.execute(
            "UPDATE cron_jobs SET next_run = ?1 WHERE id = ?2",
            params![future.to_rfc3339(), job.id],
        )?;
        Ok(())
    })
    .unwrap();

    let enabled = update_job(
        &config,
        &job.id,
        CronJobPatch {
            enabled: Some(true),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    assert_eq!(
        enabled.next_run.to_rfc3339(),
        future.to_rfc3339(),
        "enabling a job whose next_run is in the future must not reschedule it"
    );
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

// ── dedup_named_jobs ─────────────────────────────────────────────

#[test]
fn dedup_named_jobs_no_op_on_empty_db() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let removed = dedup_named_jobs(&config).unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn dedup_named_jobs_no_op_when_no_duplicates() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    add_shell_job(
        &config,
        Some("job_a".into()),
        Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo a",
    )
    .unwrap();
    add_shell_job(
        &config,
        Some("job_b".into()),
        Schedule::Cron {
            expr: "*/10 * * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo b",
    )
    .unwrap();
    let removed = dedup_named_jobs(&config).unwrap();
    assert_eq!(removed, 0);
    assert_eq!(list_jobs(&config).unwrap().len(), 2);
}

#[test]
fn dedup_named_jobs_removes_duplicates_keeping_history() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Insert two jobs with the same name directly — simulating the old double-seed bug.
    let job_a = add_shell_job(
        &config,
        Some("morning_briefing".into()),
        Schedule::Cron {
            expr: "0 7 * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo briefing",
    )
    .unwrap();
    let job_b = add_shell_job(
        &config,
        Some("morning_briefing".into()),
        Schedule::Cron {
            expr: "0 7 * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo briefing",
    )
    .unwrap();

    // Add run history to job_a — it should survive.
    let now = Utc::now();
    record_run(
        &config,
        &job_a.id,
        now,
        now + ChronoDuration::seconds(1),
        "ok",
        Some("output"),
        1000,
    )
    .unwrap();

    let removed = dedup_named_jobs(&config).unwrap();
    assert_eq!(removed, 1);

    let remaining = list_jobs(&config).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].id, job_a.id,
        "job with run history should be kept"
    );
    assert!(
        get_job(&config, &job_b.id).is_err(),
        "duplicate without history should be removed"
    );
}

#[test]
fn dedup_named_jobs_keeps_earliest_when_history_tied() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Both jobs have no run history — tie broken by earliest created_at.
    let job_a = add_shell_job(
        &config,
        Some("routine".into()),
        Schedule::Cron {
            expr: "0 8 * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo first",
    )
    .unwrap();
    let job_b = add_shell_job(
        &config,
        Some("routine".into()),
        Schedule::Cron {
            expr: "0 8 * * *".into(),
            tz: None,
            active_hours: None,
        },
        "echo second",
    )
    .unwrap();

    let removed = dedup_named_jobs(&config).unwrap();
    assert_eq!(removed, 1);

    let remaining = list_jobs(&config).unwrap();
    assert_eq!(remaining.len(), 1);
    // job_a was created first — it should win the tie.
    assert_eq!(remaining[0].id, job_a.id, "earliest job should be kept");
    assert!(get_job(&config, &job_b.id).is_err());
}

// ── add_flow_schedule_job race-safety (CodeRabbit finding A) ────────

#[test]
fn add_flow_schedule_job_twice_yields_a_single_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let schedule = Schedule::Cron {
        expr: "0 9 * * *".into(),
        tz: None,
        active_hours: None,
    };

    let first = add_flow_schedule_job(&config, "flow-1", schedule.clone()).unwrap();
    let second = add_flow_schedule_job(&config, "flow-1", schedule).unwrap();

    // Calling it twice for the same flow must not create a duplicate — the
    // second call returns the same row the first one created.
    assert_eq!(first.id, second.id);

    let flow_jobs: Vec<_> = list_jobs(&config)
        .unwrap()
        .into_iter()
        .filter(|j| j.job_type == JobType::Flow && j.command == "flow-1")
        .collect();
    assert_eq!(
        flow_jobs.len(),
        1,
        "exactly one job_type='flow' row should exist for flow-1"
    );
}

#[test]
fn add_flow_schedule_job_unique_index_does_not_affect_shell_jobs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Two shell jobs sharing the same command must both persist — the new
    // partial unique index is scoped to job_type = 'flow' and must not
    // constrain shell/agent jobs, which may legitimately share a command.
    let shell_a = add_job(&config, "*/5 * * * *", "echo shared").unwrap();
    let shell_b = add_job(&config, "*/10 * * * *", "echo shared").unwrap();

    assert!(get_job(&config, &shell_a.id).is_ok());
    assert!(get_job(&config, &shell_b.id).is_ok());
    assert_eq!(list_jobs(&config).unwrap().len(), 2);
}

#[test]
fn dedup_named_jobs_ignores_unnamed_jobs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Unnamed jobs (name = NULL) — dedup should not touch them.
    add_job(&config, "*/5 * * * *", "echo unnamed-1").unwrap();
    add_job(&config, "*/5 * * * *", "echo unnamed-2").unwrap();

    let removed = dedup_named_jobs(&config).unwrap();
    assert_eq!(removed, 0);
    assert_eq!(list_jobs(&config).unwrap().len(), 2);
}

/// Regression: gating the DDL behind a per-path "already initialized" set
/// (see [`INITIALIZED_SCHEMAS`]) must not cost the store its self-healing.
///
/// Before the gate existed, the DDL ran on every `with_connection` call, so a
/// database deleted or replaced at runtime (a workspace reset, a manual
/// deletion, a disk-recovery restore) recovered on the very next call —
/// `Connection::open` creates a fresh empty file and `CREATE TABLE IF NOT
/// EXISTS` repopulates it. With a naive cache the set still reports
/// "initialized" while the file behind it is empty, and every query afterwards
/// fails `no such table: cron_jobs` until the process restarts. This pins the
/// verify-on-hit in `ensure_schema_initialized` that restores it.
#[test]
fn schema_reinitializes_when_the_database_file_is_deleted_at_runtime() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First use populates the per-path cache and creates the schema.
    add_job(&config, "*/5 * * * *", "echo before-deletion").unwrap();
    assert_eq!(
        list_jobs(&config).unwrap().len(),
        1,
        "sanity: the job was persisted"
    );

    // Simulate a workspace reset / manual deletion while the process lives on.
    let db_path = config.workspace_dir.join("cron").join("jobs.db");
    assert!(
        db_path.exists(),
        "sanity: the cron db exists before deletion"
    );
    std::fs::remove_file(&db_path).unwrap();
    // Defensive: drop any journal sidecars too, so SQLite cannot resurrect
    // pages from them (cron uses the default rollback journal, not WAL, so
    // these normally do not exist between calls — removing them is a no-op).
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    let _ = std::fs::remove_file(db_path.with_extension("db-journal"));

    // The cache still says this path is initialized. Without the verify-on-hit
    // this errors with `no such table: cron_jobs`.
    let after = list_jobs(&config)
        .expect("a deleted database must be re-initialized, not left wedged at 'no such table'");
    assert!(
        after.is_empty(),
        "the recreated database starts empty — the prior job is genuinely gone"
    );

    // And the store is fully usable again, not merely readable.
    let recreated = add_job(&config, "*/5 * * * *", "echo after-deletion").unwrap();
    assert_eq!(
        get_job(&config, &recreated.id).unwrap().command,
        "echo after-deletion"
    );
    assert_eq!(list_jobs(&config).unwrap().len(), 1);
}

/// Regression (CodeRabbit / Codex on #5708): a cache hit must be validated
/// against the *whole* schema, not just the presence of one table. A database
/// replaced at runtime with an older/partial schema — `cron_jobs` present but a
/// migrated column dropped — must be re-migrated, not trusted and then failed on
/// the incomplete schema. The `PRAGMA user_version` check is what catches it.
#[test]
fn older_on_disk_schema_under_a_cached_path_is_remigrated() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // First use creates the full (versioned) schema and caches the path.
    let original = add_job(&config, "*/5 * * * *", "echo v1").unwrap();

    // Simulate a workspace restore of an OLDER database swapped in under the
    // same (already-cached) path: drop a migrated column and clear the version
    // stamp, exactly as a pre-migration database would look on disk.
    let db_path = config.workspace_dir.join("cron").join("jobs.db");
    {
        let raw = rusqlite::Connection::open(&db_path).unwrap();
        raw.execute_batch(
            "ALTER TABLE cron_jobs DROP COLUMN profile_id;
             PRAGMA user_version = 0;",
        )
        .unwrap();
    }

    // The path is still cached. With a single-table `sqlite_master` probe this
    // would be trusted and `list_jobs` (which selects `profile_id`) would fail
    // with `no such column`. The version check detects the drift and re-migrates.
    let listed = list_jobs(&config)
        .expect("an older on-disk schema under a cached path must be re-migrated, not trusted");
    assert_eq!(
        listed.len(),
        1,
        "the pre-existing row survives DROP COLUMN and the schema is repaired"
    );
    assert_eq!(listed[0].id, original.id);
    // The migrated column is back (reads as None for the pre-existing row).
    assert_eq!(get_job(&config, &original.id).unwrap().profile_id, None);

    // And the store is fully usable again.
    let recreated = add_job(&config, "*/5 * * * *", "echo v2").unwrap();
    assert_eq!(get_job(&config, &recreated.id).unwrap().command, "echo v2");
}
