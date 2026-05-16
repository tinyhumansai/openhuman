use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::types::{
    CanvasTask, CanvasTrackerSettings, LocalStatus, ReminderRecommendation, SyncSummary,
    UrgencyLevel,
};

const DB_DIR: &str = "canvas_tracker";
const DB_FILE: &str = "canvas_tracker.db";
const SETTINGS_KEY: &str = "settings";

const SCHEMA_DDL: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS canvas_tracker_settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS canvas_tracker_tasks (
        course_id               TEXT NOT NULL,
        assignment_id           TEXT NOT NULL,
        course_name             TEXT NOT NULL,
        assignment_name         TEXT NOT NULL,
        due_at                  TEXT,
        due_at_unclear          INTEGER NOT NULL DEFAULT 0,
        instructions_summary    TEXT NOT NULL DEFAULT '',
        submission_type         TEXT,
        canvas_workflow_state   TEXT,
        canvas_submission_state TEXT,
        local_status            TEXT NOT NULL DEFAULT 'not_started',
        urgency_level           TEXT NOT NULL DEFAULT 'unclear',
        recommended_start_at    TEXT,
        reminders_needed_json   TEXT NOT NULL DEFAULT '[]',
        source_url              TEXT,
        last_seen_at            TEXT NOT NULL,
        PRIMARY KEY (course_id, assignment_id)
    );
    CREATE INDEX IF NOT EXISTS idx_canvas_tracker_tasks_due
        ON canvas_tracker_tasks(due_at);

    CREATE TABLE IF NOT EXISTS canvas_tracker_sync_runs (
        id                       TEXT PRIMARY KEY,
        synced                   INTEGER NOT NULL,
        courses_seen             INTEGER NOT NULL,
        courses_used             INTEGER NOT NULL,
        courses_ignored          INTEGER NOT NULL,
        assignments_seen         INTEGER NOT NULL,
        tasks_upserted           INTEGER NOT NULL,
        previous_tasks_preserved INTEGER NOT NULL,
        errors_json              TEXT NOT NULL DEFAULT '[]',
        synced_at                TEXT NOT NULL
    );
";

pub struct CanvasTrackerStore {
    conn: Connection,
}

impl CanvasTrackerStore {
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        let db_path = workspace_dir.join(DB_DIR).join(DB_FILE);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "[canvas_tracker::store] failed to create {}",
                    parent.display()
                )
            })?;
        }

        let conn = Connection::open(&db_path).with_context(|| {
            format!(
                "[canvas_tracker::store] failed to open DB at {}",
                db_path.display()
            )
        })?;
        conn.execute_batch(SCHEMA_DDL)
            .context("[canvas_tracker::store] schema migration failed")?;

        let store = Self { conn };
        store.ensure_default_settings()?;
        Ok(store)
    }

    pub fn get_settings(&self) -> Result<CanvasTrackerSettings> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM canvas_tracker_settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()
            .context("[canvas_tracker::store] failed to load settings")?;

        match value {
            Some(value) => serde_json::from_str(&value)
                .context("[canvas_tracker::store] failed to parse settings JSON"),
            None => {
                let settings = CanvasTrackerSettings::default();
                self.save_settings(&settings)?;
                Ok(settings)
            }
        }
    }

    pub fn save_settings(&self, settings: &CanvasTrackerSettings) -> Result<()> {
        let value = serde_json::to_string(settings)
            .context("[canvas_tracker::store] failed to serialize settings")?;
        self.conn
            .execute(
                "INSERT INTO canvas_tracker_settings (key, value)
                 VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SETTINGS_KEY, value],
            )
            .context("[canvas_tracker::store] failed to save settings")?;
        Ok(())
    }

    pub fn upsert_tasks(&self, tasks: &[CanvasTask]) -> Result<usize> {
        if tasks.is_empty() {
            return Ok(0);
        }

        let tx = self
            .conn
            .unchecked_transaction()
            .context("[canvas_tracker::store] failed to begin task upsert transaction")?;
        let mut upserted = 0;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO canvas_tracker_tasks (
                        course_id, assignment_id, course_name, assignment_name, due_at,
                        due_at_unclear, instructions_summary, submission_type,
                        canvas_workflow_state, canvas_submission_state, local_status,
                        urgency_level, recommended_start_at, reminders_needed_json,
                        source_url, last_seen_at
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                        ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
                     )
                     ON CONFLICT(course_id, assignment_id) DO UPDATE SET
                        course_name = excluded.course_name,
                        assignment_name = excluded.assignment_name,
                        due_at = excluded.due_at,
                        due_at_unclear = excluded.due_at_unclear,
                        instructions_summary = excluded.instructions_summary,
                        submission_type = excluded.submission_type,
                        canvas_workflow_state = excluded.canvas_workflow_state,
                        canvas_submission_state = excluded.canvas_submission_state,
                        local_status = CASE
                            WHEN excluded.local_status = 'submitted' THEN excluded.local_status
                            ELSE canvas_tracker_tasks.local_status
                        END,
                        urgency_level = excluded.urgency_level,
                        recommended_start_at = excluded.recommended_start_at,
                        reminders_needed_json = excluded.reminders_needed_json,
                        source_url = excluded.source_url,
                        last_seen_at = excluded.last_seen_at",
                )
                .context("[canvas_tracker::store] failed to prepare task upsert")?;

            for task in tasks {
                let reminders_needed_json = serde_json::to_string(&task.reminders_needed)
                    .context("[canvas_tracker::store] failed to serialize reminders")?;
                upserted += stmt
                    .execute(params![
                        &task.course_id,
                        &task.assignment_id,
                        &task.course_name,
                        &task.assignment_name,
                        &task.due_at,
                        if task.due_at_unclear { 1 } else { 0 },
                        &task.instructions_summary,
                        &task.submission_type,
                        &task.canvas_workflow_state,
                        &task.canvas_submission_state,
                        task.local_status.as_str(),
                        urgency_level_as_str(task.urgency_level),
                        &task.recommended_start_at,
                        reminders_needed_json,
                        &task.source_url,
                        &task.last_seen_at,
                    ])
                    .context("[canvas_tracker::store] failed to upsert task")?;
            }
        }
        tx.commit()
            .context("[canvas_tracker::store] failed to commit task upsert transaction")?;
        Ok(upserted)
    }

    pub fn list_tasks(&self) -> Result<Vec<CanvasTask>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT course_id, course_name, assignment_id, assignment_name, due_at,
                        due_at_unclear, instructions_summary, submission_type,
                        canvas_workflow_state, canvas_submission_state, local_status,
                        urgency_level, recommended_start_at, reminders_needed_json,
                        source_url, last_seen_at
                 FROM canvas_tracker_tasks
                 ORDER BY due_at IS NULL, due_at ASC, course_name ASC, assignment_name ASC",
            )
            .context("[canvas_tracker::store] failed to prepare task list")?;

        let rows = stmt.query_map([], map_task_row)?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    pub fn update_local_status(
        &self,
        course_id: &str,
        assignment_id: &str,
        status: LocalStatus,
    ) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE canvas_tracker_tasks
                 SET local_status = ?1
                 WHERE course_id = ?2 AND assignment_id = ?3",
                params![status.as_str(), course_id, assignment_id],
            )
            .context("[canvas_tracker::store] failed to update local status")?;
        if changed == 0 {
            anyhow::bail!(
                "[canvas_tracker::store] task not found for course_id={course_id} assignment_id={assignment_id}"
            );
        }
        Ok(())
    }

    pub fn record_sync_run(&self, summary: &SyncSummary) -> Result<()> {
        let errors_json = serde_json::to_string(&summary.errors)
            .context("[canvas_tracker::store] failed to serialize sync errors")?;
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO canvas_tracker_sync_runs (
                    id, synced, courses_seen, courses_used, courses_ignored,
                    assignments_seen, tasks_upserted, previous_tasks_preserved,
                    errors_json, synced_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    if summary.synced { 1 } else { 0 },
                    summary.courses_seen as i64,
                    summary.courses_used as i64,
                    summary.courses_ignored as i64,
                    summary.assignments_seen as i64,
                    summary.tasks_upserted as i64,
                    if summary.previous_tasks_preserved {
                        1
                    } else {
                        0
                    },
                    errors_json,
                    &summary.synced_at,
                ],
            )
            .context("[canvas_tracker::store] failed to record sync run")?;
        Ok(())
    }

    fn ensure_default_settings(&self) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO canvas_tracker_settings (key, value) VALUES (?1, ?2)",
                params![
                    SETTINGS_KEY,
                    serde_json::to_string(&CanvasTrackerSettings::default())?
                ],
            )
            .context("[canvas_tracker::store] failed to seed default settings")?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn sync_run_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM canvas_tracker_sync_runs", [], |row| {
                row.get(0)
            })
            .context("[canvas_tracker::store] failed to count sync runs")?;
        Ok(count as usize)
    }
}

fn map_task_row(row: &Row<'_>) -> rusqlite::Result<CanvasTask> {
    let reminders_json: String = row.get(13)?;
    let reminders_needed = parse_reminders(&reminders_json).map_err(to_sql_error)?;
    let local_status: String = row.get(10)?;
    let urgency_level: String = row.get(11)?;
    Ok(CanvasTask {
        course_id: row.get(0)?,
        course_name: row.get(1)?,
        assignment_id: row.get(2)?,
        assignment_name: row.get(3)?,
        due_at: row.get(4)?,
        due_at_unclear: row.get::<_, i64>(5)? != 0,
        instructions_summary: row.get(6)?,
        submission_type: row.get(7)?,
        canvas_workflow_state: row.get(8)?,
        canvas_submission_state: row.get(9)?,
        local_status: parse_local_status(&local_status).map_err(to_sql_error_message)?,
        urgency_level: parse_urgency_level(&urgency_level).map_err(to_sql_error_message)?,
        recommended_start_at: row.get(12)?,
        reminders_needed,
        source_url: row.get(14)?,
        last_seen_at: row.get(15)?,
    })
}

fn parse_reminders(value: &str) -> Result<Vec<ReminderRecommendation>, serde_json::Error> {
    serde_json::from_str(value)
}

fn parse_local_status(value: &str) -> Result<LocalStatus, String> {
    match value {
        "not_started" => Ok(LocalStatus::NotStarted),
        "in_progress" => Ok(LocalStatus::InProgress),
        "waiting" => Ok(LocalStatus::Waiting),
        "submitted" => Ok(LocalStatus::Submitted),
        "done" => Ok(LocalStatus::Done),
        "unclear" => Ok(LocalStatus::Unclear),
        other => Err(format!("unknown local status: {other}")),
    }
}

fn urgency_level_as_str(value: UrgencyLevel) -> &'static str {
    match value {
        UrgencyLevel::Critical => "critical",
        UrgencyLevel::High => "high",
        UrgencyLevel::Medium => "medium",
        UrgencyLevel::Low => "low",
        UrgencyLevel::Unclear => "unclear",
    }
}

fn parse_urgency_level(value: &str) -> Result<UrgencyLevel, String> {
    match value {
        "critical" => Ok(UrgencyLevel::Critical),
        "high" => Ok(UrgencyLevel::High),
        "medium" => Ok(UrgencyLevel::Medium),
        "low" => Ok(UrgencyLevel::Low),
        "unclear" => Ok(UrgencyLevel::Unclear),
        other => Err(format!("unknown urgency level: {other}")),
    }
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn to_sql_error_message(error: String) -> rusqlite::Error {
    to_sql_error(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
