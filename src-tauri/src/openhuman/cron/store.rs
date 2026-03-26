use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    next_run_for_schedule, schedule_cron_expression, validate_schedule, CronJob, CronJobPatch,
    CronRun, DeliveryConfig, JobType, Schedule, SessionTarget,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

const MAX_CRON_OUTPUT_BYTES: usize = 16 * 1024;
const TRUNCATED_OUTPUT_MARKER: &str = "\n...[truncated]";

pub fn add_job(config: &Config, expression: &str, command: &str) -> Result<CronJob> {
    let schedule = Schedule::Cron {
        expr: expression.to_string(),
        tz: None,
    };
    add_shell_job(config, None, schedule, command)
}

pub fn add_shell_job(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    command: &str,
) -> Result<CronJob> {
    let now = Utc::now();
    validate_schedule(&schedule, now)?;
    let next_run = next_run_for_schedule(&schedule, now)?;
    let id = Uuid::new_v4().to_string();
    let expression = schedule_cron_expression(&schedule).unwrap_or_default();
    let schedule_json = serde_json::to_string(&schedule)?;

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO cron_jobs (
                id, expression, command, schedule, job_type, prompt, name, session_target, model,
                enabled, delivery, delete_after_run, created_at, next_run
             ) VALUES (?1, ?2, ?3, ?4, 'shell', NULL, ?5, 'isolated', NULL, 1, ?6, 0, ?7, ?8)",
            params![
                id,
                expression,
                command,
                schedule_json,
                name,
                serde_json::to_string(&DeliveryConfig::default())?,
                now.to_rfc3339(),
                next_run.to_rfc3339(),
            ],
        )
        .context("Failed to insert cron shell job")?;
        Ok(())
    })?;

    get_job(config, &id)
}

#[allow(clippy::too_many_arguments)]
pub fn add_agent_job(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    prompt: &str,
    session_target: SessionTarget,
    model: Option<String>,
    delivery: Option<DeliveryConfig>,
    delete_after_run: bool,
) -> Result<CronJob> {
    let now = Utc::now();
    validate_schedule(&schedule, now)?;
    let next_run = next_run_for_schedule(&schedule, now)?;
    let id = Uuid::new_v4().to_string();
    let expression = schedule_cron_expression(&schedule).unwrap_or_default();
    let schedule_json = serde_json::to_string(&schedule)?;
    let delivery = delivery.unwrap_or_default();

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO cron_jobs (
                id, expression, command, schedule, job_type, prompt, name, session_target, model,
                enabled, delivery, delete_after_run, created_at, next_run
             ) VALUES (?1, ?2, '', ?3, 'agent', ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11)",
            params![
                id,
                expression,
                schedule_json,
                prompt,
                name,
                session_target.as_str(),
                model,
                serde_json::to_string(&delivery)?,
                if delete_after_run { 1 } else { 0 },
                now.to_rfc3339(),
                next_run.to_rfc3339(),
            ],
        )
        .context("Failed to insert cron agent job")?;
        Ok(())
    })?;

    get_job(config, &id)
}

pub fn list_jobs(config: &Config) -> Result<Vec<CronJob>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status, last_output
             FROM cron_jobs ORDER BY next_run ASC",
        )?;

        let rows = stmt.query_map([], map_cron_job_row)?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    })
}

pub fn get_job(config: &Config, job_id: &str) -> Result<CronJob> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status, last_output
             FROM cron_jobs WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![job_id])?;
        if let Some(row) = rows.next()? {
            map_cron_job_row(row).map_err(Into::into)
        } else {
            anyhow::bail!("Cron job '{job_id}' not found")
        }
    })
}

pub fn remove_job(config: &Config, id: &str) -> Result<()> {
    let changed = with_connection(config, |conn| {
        conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])
            .context("Failed to delete cron job")
    })?;

    if changed == 0 {
        anyhow::bail!("Cron job '{id}' not found");
    }

    println!("✅ Removed cron job {id}");
    Ok(())
}

pub fn due_jobs(config: &Config, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
    let lim = i64::try_from(config.scheduler.max_tasks.max(1))
        .context("Scheduler max_tasks overflows i64")?;
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, expression, command, schedule, job_type, prompt, name, session_target, model,
                    enabled, delivery, delete_after_run, created_at, next_run, last_run, last_status, last_output
             FROM cron_jobs
             WHERE enabled = 1 AND next_run <= ?1
             ORDER BY next_run ASC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![now.to_rfc3339(), lim], map_cron_job_row)?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    })
}

pub fn update_job(config: &Config, job_id: &str, patch: CronJobPatch) -> Result<CronJob> {
    let mut job = get_job(config, job_id)?;
    let mut schedule_changed = false;

    if let Some(schedule) = patch.schedule {
        validate_schedule(&schedule, Utc::now())?;
        job.schedule = schedule;
        job.expression = schedule_cron_expression(&job.schedule).unwrap_or_default();
        schedule_changed = true;
    }
    if let Some(command) = patch.command {
        job.command = command;
    }
    if let Some(prompt) = patch.prompt {
        job.prompt = Some(prompt);
    }
    if let Some(name) = patch.name {
        job.name = Some(name);
    }
    if let Some(enabled) = patch.enabled {
        job.enabled = enabled;
    }
    if let Some(delivery) = patch.delivery {
        job.delivery = delivery;
    }
    if let Some(model) = patch.model {
        job.model = Some(model);
    }
    if let Some(target) = patch.session_target {
        job.session_target = target;
    }
    if let Some(delete_after_run) = patch.delete_after_run {
        job.delete_after_run = delete_after_run;
    }

    if schedule_changed {
        job.next_run = next_run_for_schedule(&job.schedule, Utc::now())?;
    }

    with_connection(config, |conn| {
        conn.execute(
            "UPDATE cron_jobs
             SET expression = ?1, command = ?2, schedule = ?3, job_type = ?4, prompt = ?5, name = ?6,
                 session_target = ?7, model = ?8, enabled = ?9, delivery = ?10, delete_after_run = ?11,
                 next_run = ?12
             WHERE id = ?13",
            params![
                job.expression,
                job.command,
                serde_json::to_string(&job.schedule)?,
                job.job_type.as_str(),
                job.prompt,
                job.name,
                job.session_target.as_str(),
                job.model,
                if job.enabled { 1 } else { 0 },
                serde_json::to_string(&job.delivery)?,
                if job.delete_after_run { 1 } else { 0 },
                job.next_run.to_rfc3339(),
                job.id,
            ],
        )
        .context("Failed to update cron job")?;
        Ok(())
    })?;

    get_job(config, job_id)
}

pub fn record_last_run(
    config: &Config,
    job_id: &str,
    finished_at: DateTime<Utc>,
    success: bool,
    output: &str,
) -> Result<()> {
    let status = if success { "ok" } else { "error" };
    let bounded_output = truncate_cron_output(output);
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE cron_jobs
             SET last_run = ?1, last_status = ?2, last_output = ?3
             WHERE id = ?4",
            params![finished_at.to_rfc3339(), status, bounded_output, job_id],
        )
        .context("Failed to update cron last run fields")?;
        Ok(())
    })
}

pub fn reschedule_after_run(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
) -> Result<()> {
    let now = Utc::now();
    let next_run = next_run_for_schedule(&job.schedule, now)?;
    let status = if success { "ok" } else { "error" };
    let bounded_output = truncate_cron_output(output);

    with_connection(config, |conn| {
        conn.execute(
            "UPDATE cron_jobs
             SET next_run = ?1, last_run = ?2, last_status = ?3, last_output = ?4
             WHERE id = ?5",
            params![
                next_run.to_rfc3339(),
                now.to_rfc3339(),
                status,
                bounded_output,
                job.id
            ],
        )
        .context("Failed to update cron job run state")?;
        Ok(())
    })
}

pub fn record_run(
    config: &Config,
    job_id: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    status: &str,
    output: Option<&str>,
    duration_ms: i64,
) -> Result<()> {
    let bounded_output = output.map(truncate_cron_output);
    with_connection(config, |conn| {
        // Wrap INSERT + pruning DELETE in an explicit transaction so that
        // if the DELETE fails, the INSERT is rolled back and the run table
        // cannot grow unboundedly.
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO cron_runs (job_id, started_at, finished_at, status, output, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                job_id,
                started_at.to_rfc3339(),
                finished_at.to_rfc3339(),
                status,
                bounded_output.as_deref(),
                duration_ms,
            ],
        )
        .context("Failed to insert cron run")?;

        let keep = config.cron.max_run_history.max(1) as i64;
        tx.execute(
            "DELETE FROM cron_runs
             WHERE job_id = ?1
               AND id NOT IN (
                 SELECT id FROM cron_runs
                 WHERE job_id = ?1
                 ORDER BY started_at DESC, id DESC
                 LIMIT ?2
               )",
            params![job_id, keep],
        )
        .context("Failed to prune cron run history")?;

        tx.commit()
            .context("Failed to commit cron run transaction")?;
        Ok(())
    })
}

fn truncate_cron_output(output: &str) -> String {
    if output.len() <= MAX_CRON_OUTPUT_BYTES {
        return output.to_string();
    }

    if MAX_CRON_OUTPUT_BYTES <= TRUNCATED_OUTPUT_MARKER.len() {
        return TRUNCATED_OUTPUT_MARKER.to_string();
    }

    let mut cutoff = MAX_CRON_OUTPUT_BYTES - TRUNCATED_OUTPUT_MARKER.len();
    while cutoff > 0 && !output.is_char_boundary(cutoff) {
        cutoff -= 1;
    }

    let mut truncated = output[..cutoff].to_string();
    truncated.push_str(TRUNCATED_OUTPUT_MARKER);
    truncated
}

pub fn list_runs(config: &Config, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
    with_connection(config, |conn| {
        let lim = i64::try_from(limit.max(1)).context("Run history limit overflow")?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, started_at, finished_at, status, output, duration_ms
             FROM cron_runs
             WHERE job_id = ?1
             ORDER BY started_at DESC, id DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![job_id, lim], |row| {
            Ok(CronRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: parse_rfc3339(&row.get::<_, String>(2)?)
                    .map_err(sql_conversion_error)?,
                finished_at: parse_rfc3339(&row.get::<_, String>(3)?)
                    .map_err(sql_conversion_error)?,
                status: row.get(4)?,
                output: row.get(5)?,
                duration_ms: row.get(6)?,
            })
        })?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    })
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("Invalid RFC3339 timestamp in cron DB: {raw}"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn sql_conversion_error(err: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(err.into())
}

fn map_cron_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    let expression: String = row.get(1)?;
    let schedule_raw: Option<String> = row.get(3)?;
    let schedule =
        decode_schedule(schedule_raw.as_deref(), &expression).map_err(sql_conversion_error)?;

    let delivery_raw: Option<String> = row.get(10)?;
    let delivery = decode_delivery(delivery_raw.as_deref()).map_err(sql_conversion_error)?;

    let next_run_raw: String = row.get(13)?;
    let last_run_raw: Option<String> = row.get(14)?;
    let created_at_raw: String = row.get(12)?;

    Ok(CronJob {
        id: row.get(0)?,
        expression,
        schedule,
        command: row.get(2)?,
        job_type: JobType::parse(&row.get::<_, String>(4)?),
        prompt: row.get(5)?,
        name: row.get(6)?,
        session_target: SessionTarget::parse(&row.get::<_, String>(7)?),
        model: row.get(8)?,
        enabled: row.get::<_, i64>(9)? != 0,
        delivery,
        delete_after_run: row.get::<_, i64>(11)? != 0,
        created_at: parse_rfc3339(&created_at_raw).map_err(sql_conversion_error)?,
        next_run: parse_rfc3339(&next_run_raw).map_err(sql_conversion_error)?,
        last_run: match last_run_raw {
            Some(raw) => Some(parse_rfc3339(&raw).map_err(sql_conversion_error)?),
            None => None,
        },
        last_status: row.get(15)?,
        last_output: row.get(16)?,
    })
}

fn decode_schedule(schedule_raw: Option<&str>, expression: &str) -> Result<Schedule> {
    if let Some(raw) = schedule_raw {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return serde_json::from_str(trimmed)
                .with_context(|| format!("Failed to parse cron schedule JSON: {trimmed}"));
        }
    }

    if expression.trim().is_empty() {
        anyhow::bail!("Missing schedule and legacy expression for cron job")
    }

    Ok(Schedule::Cron {
        expr: expression.to_string(),
        tz: None,
    })
}

fn decode_delivery(delivery_raw: Option<&str>) -> Result<DeliveryConfig> {
    if let Some(raw) = delivery_raw {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return serde_json::from_str(trimmed)
                .with_context(|| format!("Failed to parse cron delivery JSON: {trimmed}"));
        }
    }
    Ok(DeliveryConfig::default())
}

fn add_column_if_missing(conn: &Connection, name: &str, sql_type: &str) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(cron_jobs)")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(1)?;
        if col_name == name {
            return Ok(());
        }
    }
    // Drop the statement/rows before executing ALTER to release any locks
    drop(rows);
    drop(stmt);

    // Tolerate "duplicate column name" errors to handle the race where
    // another process adds the column between our PRAGMA check and ALTER.
    match conn.execute(
        &format!("ALTER TABLE cron_jobs ADD COLUMN {name} {sql_type}"),
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(err, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            tracing::debug!("Column cron_jobs.{name} already exists (concurrent migration): {err}");
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("Failed to add cron_jobs.{name}")),
    }
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("cron").join("jobs.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cron directory: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open cron DB: {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS cron_jobs (
            id               TEXT PRIMARY KEY,
            expression       TEXT NOT NULL,
            command          TEXT NOT NULL,
            schedule         TEXT,
            job_type         TEXT NOT NULL DEFAULT 'shell',
            prompt           TEXT,
            name             TEXT,
            session_target   TEXT NOT NULL DEFAULT 'isolated',
            model            TEXT,
            enabled          INTEGER NOT NULL DEFAULT 1,
            delivery         TEXT,
            delete_after_run INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT NOT NULL,
            next_run         TEXT NOT NULL,
            last_run         TEXT,
            last_status      TEXT,
            last_output      TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run);

        CREATE TABLE IF NOT EXISTS cron_runs (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id      TEXT NOT NULL,
            started_at  TEXT NOT NULL,
            finished_at TEXT NOT NULL,
            status      TEXT NOT NULL,
            output      TEXT,
            duration_ms INTEGER,
            FOREIGN KEY (job_id) REFERENCES cron_jobs(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_cron_runs_job_id ON cron_runs(job_id);
        CREATE INDEX IF NOT EXISTS idx_cron_runs_started_at ON cron_runs(started_at);
        CREATE INDEX IF NOT EXISTS idx_cron_runs_job_started ON cron_runs(job_id, started_at);",
    )
    .context("Failed to initialize cron schema")?;

    add_column_if_missing(&conn, "schedule", "TEXT")?;
    add_column_if_missing(&conn, "job_type", "TEXT NOT NULL DEFAULT 'shell'")?;
    add_column_if_missing(&conn, "prompt", "TEXT")?;
    add_column_if_missing(&conn, "name", "TEXT")?;
    add_column_if_missing(&conn, "session_target", "TEXT NOT NULL DEFAULT 'isolated'")?;
    add_column_if_missing(&conn, "model", "TEXT")?;
    add_column_if_missing(&conn, "enabled", "INTEGER NOT NULL DEFAULT 1")?;
    add_column_if_missing(&conn, "delivery", "TEXT")?;
    add_column_if_missing(&conn, "delete_after_run", "INTEGER NOT NULL DEFAULT 0")?;

    f(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
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
}
