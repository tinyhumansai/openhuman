//! SQLite persistence for subconscious tasks, execution log, and escalations.
//!
//! Follows the cron module's `with_connection` pattern: opens the database,
//! runs DDL on every connection, and provides pure functions.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

use super::types::{
    Escalation, EscalationPriority, EscalationStatus, SubconsciousLogEntry, SubconsciousTask,
    TaskPatch, TaskRecurrence, TaskSource,
};

/// Open the subconscious database and run schema migrations.
pub fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("subconscious").join("subconscious.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create subconscious dir: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open subconscious DB: {}", db_path.display()))?;

    conn.execute_batch(SCHEMA_DDL)
        .with_context(|| "failed to run subconscious schema DDL")?;

    f(&conn)
}

const SCHEMA_DDL: &str = "
    PRAGMA foreign_keys = ON;
    PRAGMA journal_mode = WAL;

    CREATE TABLE IF NOT EXISTS subconscious_tasks (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        source      TEXT NOT NULL DEFAULT 'user',
        recurrence  TEXT NOT NULL DEFAULT 'pending',
        enabled     INTEGER NOT NULL DEFAULT 1,
        last_run_at REAL,
        next_run_at REAL,
        completed   INTEGER NOT NULL DEFAULT 0,
        created_at  REAL NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_tasks_next_run
        ON subconscious_tasks(next_run_at);
    CREATE INDEX IF NOT EXISTS idx_tasks_enabled
        ON subconscious_tasks(enabled, completed);

    CREATE TABLE IF NOT EXISTS subconscious_log (
        id          TEXT PRIMARY KEY,
        task_id     TEXT NOT NULL,
        tick_at     REAL NOT NULL,
        decision    TEXT NOT NULL,
        result      TEXT,
        duration_ms INTEGER,
        created_at  REAL NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_log_task
        ON subconscious_log(task_id, tick_at DESC);
    CREATE INDEX IF NOT EXISTS idx_log_tick
        ON subconscious_log(tick_at DESC);

    CREATE TABLE IF NOT EXISTS subconscious_escalations (
        id          TEXT PRIMARY KEY,
        task_id     TEXT NOT NULL,
        log_id      TEXT,
        title       TEXT NOT NULL,
        description TEXT NOT NULL,
        priority    TEXT NOT NULL DEFAULT 'normal',
        status      TEXT NOT NULL DEFAULT 'pending',
        created_at  REAL NOT NULL,
        resolved_at REAL
    );
    CREATE INDEX IF NOT EXISTS idx_escalations_status
        ON subconscious_escalations(status);
";

// ── Task CRUD ────────────────────────────────────────────────────────────────

pub fn add_task(
    conn: &Connection,
    title: &str,
    source: TaskSource,
    recurrence: TaskRecurrence,
) -> Result<SubconsciousTask> {
    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    let source_str = serde_json::to_value(&source)
        .unwrap_or_default()
        .as_str()
        .unwrap_or("user")
        .to_string();
    let recurrence_str = recurrence_to_string(&recurrence);

    conn.execute(
        "INSERT INTO subconscious_tasks (id, title, source, recurrence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, title, source_str, recurrence_str, now],
    )?;

    Ok(SubconsciousTask {
        id,
        title: title.to_string(),
        source,
        recurrence,
        enabled: true,
        last_run_at: None,
        next_run_at: None,
        completed: false,
        created_at: now,
    })
}

pub fn get_task(conn: &Connection, task_id: &str) -> Result<SubconsciousTask> {
    conn.query_row(
        "SELECT id, title, source, recurrence, enabled, last_run_at, next_run_at, completed, created_at
         FROM subconscious_tasks WHERE id = ?1",
        [task_id],
        row_to_task,
    )
    .with_context(|| format!("task not found: {task_id}"))
}

pub fn list_tasks(conn: &Connection, enabled_only: bool) -> Result<Vec<SubconsciousTask>> {
    let sql = if enabled_only {
        "SELECT id, title, source, recurrence, enabled, last_run_at, next_run_at, completed, created_at
         FROM subconscious_tasks WHERE enabled = 1 ORDER BY created_at"
    } else {
        "SELECT id, title, source, recurrence, enabled, last_run_at, next_run_at, completed, created_at
         FROM subconscious_tasks ORDER BY created_at"
    };
    let mut stmt = conn.prepare(sql)?;
    let tasks = stmt
        .query_map([], row_to_task)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tasks)
}

pub fn update_task(conn: &Connection, task_id: &str, patch: &TaskPatch) -> Result<()> {
    if let Some(ref title) = patch.title {
        conn.execute(
            "UPDATE subconscious_tasks SET title = ?1 WHERE id = ?2",
            rusqlite::params![title, task_id],
        )?;
    }
    if let Some(ref recurrence) = patch.recurrence {
        conn.execute(
            "UPDATE subconscious_tasks SET recurrence = ?1 WHERE id = ?2",
            rusqlite::params![recurrence_to_string(recurrence), task_id],
        )?;
    }
    if let Some(enabled) = patch.enabled {
        conn.execute(
            "UPDATE subconscious_tasks SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![enabled, task_id],
        )?;
    }
    Ok(())
}

/// Remove a task. System tasks cannot be deleted — only disabled.
pub fn remove_task(conn: &Connection, task_id: &str) -> Result<()> {
    let source: String = conn
        .query_row(
            "SELECT source FROM subconscious_tasks WHERE id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .with_context(|| format!("task not found: {task_id}"))?;

    if source == "system" {
        anyhow::bail!("System tasks cannot be deleted. Disable them instead.");
    }

    conn.execute("DELETE FROM subconscious_tasks WHERE id = ?1", [task_id])?;
    Ok(())
}

/// Get tasks that are due for evaluation (enabled, not completed, due now or never run).
pub fn due_tasks(conn: &Connection, now: f64) -> Result<Vec<SubconsciousTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, source, recurrence, enabled, last_run_at, next_run_at, completed, created_at
         FROM subconscious_tasks
         WHERE enabled = 1 AND completed = 0
           AND (next_run_at IS NULL OR next_run_at <= ?1)
         ORDER BY next_run_at NULLS FIRST",
    )?;
    let tasks = stmt
        .query_map([now], row_to_task)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tasks)
}

pub fn mark_task_completed(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE subconscious_tasks SET completed = 1 WHERE id = ?1",
        [task_id],
    )?;
    Ok(())
}

pub fn update_task_run_times(
    conn: &Connection,
    task_id: &str,
    last_run_at: f64,
    next_run_at: Option<f64>,
) -> Result<()> {
    conn.execute(
        "UPDATE subconscious_tasks SET last_run_at = ?1, next_run_at = ?2 WHERE id = ?3",
        rusqlite::params![last_run_at, next_run_at, task_id],
    )?;
    Ok(())
}

pub fn task_count(conn: &Connection) -> Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM subconscious_tasks WHERE enabled = 1 AND completed = 0",
        [],
        |row| row.get::<_, u64>(0),
    )
    .map_err(Into::into)
}

// ── Log CRUD ─────────────────────────────────────────────────────────────────

pub fn add_log_entry(
    conn: &Connection,
    task_id: &str,
    tick_at: f64,
    decision: &str,
    result: Option<&str>,
    duration_ms: Option<i64>,
) -> Result<SubconsciousLogEntry> {
    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    conn.execute(
        "INSERT INTO subconscious_log (id, task_id, tick_at, decision, result, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, task_id, tick_at, decision, result, duration_ms, now],
    )?;
    Ok(SubconsciousLogEntry {
        id,
        task_id: task_id.to_string(),
        tick_at,
        decision: decision.to_string(),
        result: result.map(String::from),
        duration_ms,
        created_at: now,
    })
}

/// Update an existing log entry's decision, result, and duration in place.
pub fn update_log_entry(
    conn: &Connection,
    log_id: &str,
    decision: &str,
    result: Option<&str>,
    duration_ms: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE subconscious_log SET decision = ?1, result = ?2, duration_ms = ?3 WHERE id = ?4",
        rusqlite::params![decision, result, duration_ms, log_id],
    )?;
    Ok(())
}

/// Bulk-update ALL in_progress log entries to cancelled.
/// Any entry still in_progress when a new tick starts is stale by definition.
pub fn cancel_stale_in_progress(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "UPDATE subconscious_log SET decision = 'cancelled', result = 'Superseded by new tick'
         WHERE decision = 'in_progress'",
        [],
    )?;
    Ok(count)
}

pub fn list_log_entries(
    conn: &Connection,
    task_id: Option<&str>,
    limit: usize,
) -> Result<Vec<SubconsciousLogEntry>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(tid) = task_id {
        (
            "SELECT id, task_id, tick_at, decision, result, duration_ms, created_at
             FROM subconscious_log WHERE task_id = ?1 ORDER BY tick_at DESC LIMIT ?2",
            vec![Box::new(tid.to_string()), Box::new(limit as i64)],
        )
    } else {
        (
            "SELECT id, task_id, tick_at, decision, result, duration_ms, created_at
             FROM subconscious_log ORDER BY tick_at DESC LIMIT ?1",
            vec![Box::new(limit as i64)],
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let entries = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(SubconsciousLogEntry {
                id: row.get(0)?,
                task_id: row.get(1)?,
                tick_at: row.get(2)?,
                decision: row.get(3)?,
                result: row.get(4)?,
                duration_ms: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries)
}

// ── Escalation CRUD ──────────────────────────────────────────────────────────

pub fn add_escalation(
    conn: &Connection,
    task_id: &str,
    log_id: Option<&str>,
    title: &str,
    description: &str,
    priority: &EscalationPriority,
) -> Result<Escalation> {
    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    let priority_str = serde_json::to_value(priority)
        .unwrap_or_default()
        .as_str()
        .unwrap_or("normal")
        .to_string();
    conn.execute(
        "INSERT INTO subconscious_escalations (id, task_id, log_id, title, description, priority, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, task_id, log_id, title, description, priority_str, now],
    )?;
    Ok(Escalation {
        id,
        task_id: task_id.to_string(),
        log_id: log_id.map(String::from),
        title: title.to_string(),
        description: description.to_string(),
        priority: priority.clone(),
        status: EscalationStatus::Pending,
        created_at: now,
        resolved_at: None,
    })
}

pub fn list_escalations(
    conn: &Connection,
    status_filter: Option<&EscalationStatus>,
) -> Result<Vec<Escalation>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(status) =
        status_filter
    {
        let status_str = serde_json::to_value(status)
            .unwrap_or_default()
            .as_str()
            .unwrap_or("pending")
            .to_string();
        (
            "SELECT id, task_id, log_id, title, description, priority, status, created_at, resolved_at
             FROM subconscious_escalations WHERE status = ?1 ORDER BY created_at DESC",
            vec![Box::new(status_str)],
        )
    } else {
        (
            "SELECT id, task_id, log_id, title, description, priority, status, created_at, resolved_at
             FROM subconscious_escalations ORDER BY created_at DESC",
            vec![],
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), row_to_escalation)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn resolve_escalation(
    conn: &Connection,
    escalation_id: &str,
    status: &EscalationStatus,
) -> Result<()> {
    let now = now_secs();
    let status_str = serde_json::to_value(status)
        .unwrap_or_default()
        .as_str()
        .unwrap_or("dismissed")
        .to_string();
    conn.execute(
        "UPDATE subconscious_escalations SET status = ?1, resolved_at = ?2 WHERE id = ?3",
        rusqlite::params![status_str, now, escalation_id],
    )?;
    Ok(())
}

pub fn pending_escalation_count(conn: &Connection) -> Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM subconscious_escalations WHERE status = 'pending'",
        [],
        |row| row.get::<_, u64>(0),
    )
    .map_err(Into::into)
}

pub fn get_escalation(conn: &Connection, escalation_id: &str) -> Result<Escalation> {
    conn.query_row(
        "SELECT id, task_id, log_id, title, description, priority, status, created_at, resolved_at
         FROM subconscious_escalations WHERE id = ?1",
        [escalation_id],
        row_to_escalation,
    )
    .with_context(|| format!("escalation not found: {escalation_id}"))
}

// ── Seed default system tasks ────────────────────────────────────────────────

/// Default system tasks that are always seeded and cannot be deleted.
const DEFAULT_SYSTEM_TASKS: &[&str] = &[
    "Check connected skills for errors or disconnections",
    "Review new memory updates for actionable items",
    "Monitor system health (Ollama, memory, connections)",
];

/// Seed default system tasks into SQLite.
/// Skips tasks whose title already exists. Returns the count of newly created tasks.
pub fn seed_default_tasks(conn: &Connection) -> Result<usize> {
    let mut count = 0;

    for title in DEFAULT_SYSTEM_TASKS {
        if !task_title_exists(conn, title)? {
            add_task(conn, title, TaskSource::System, TaskRecurrence::Pending)?;
            count += 1;
        }
    }

    Ok(count)
}

fn task_title_exists(conn: &Connection, title: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM subconscious_tasks WHERE title = ?1)",
        [title],
        |row| row.get(0),
    )?)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<SubconsciousTask> {
    let source_str: String = row.get(2)?;
    let recurrence_str: String = row.get(3)?;
    Ok(SubconsciousTask {
        id: row.get(0)?,
        title: row.get(1)?,
        source: string_to_source(&source_str),
        recurrence: string_to_recurrence(&recurrence_str),
        enabled: row.get::<_, bool>(4)?,
        last_run_at: row.get(5)?,
        next_run_at: row.get(6)?,
        completed: row.get::<_, bool>(7)?,
        created_at: row.get(8)?,
    })
}

fn row_to_escalation(row: &rusqlite::Row) -> rusqlite::Result<Escalation> {
    let priority_str: String = row.get(5)?;
    let status_str: String = row.get(6)?;
    Ok(Escalation {
        id: row.get(0)?,
        task_id: row.get(1)?,
        log_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        priority: string_to_priority(&priority_str),
        status: string_to_status(&status_str),
        created_at: row.get(7)?,
        resolved_at: row.get(8)?,
    })
}

fn recurrence_to_string(r: &TaskRecurrence) -> String {
    match r {
        TaskRecurrence::Once => "once".to_string(),
        TaskRecurrence::Cron(expr) => format!("cron:{expr}"),
        TaskRecurrence::Pending => "pending".to_string(),
    }
}

fn string_to_recurrence(s: &str) -> TaskRecurrence {
    if s == "once" {
        TaskRecurrence::Once
    } else if let Some(expr) = s.strip_prefix("cron:") {
        TaskRecurrence::Cron(expr.to_string())
    } else {
        TaskRecurrence::Pending
    }
}

fn string_to_source(s: &str) -> TaskSource {
    match s {
        "system" => TaskSource::System,
        _ => TaskSource::User,
    }
}

fn string_to_priority(s: &str) -> EscalationPriority {
    match s {
        "critical" => EscalationPriority::Critical,
        "important" => EscalationPriority::Important,
        _ => EscalationPriority::Normal,
    }
}

fn string_to_status(s: &str) -> EscalationStatus {
    match s {
        "approved" => EscalationStatus::Approved,
        "dismissed" => EscalationStatus::Dismissed,
        _ => EscalationStatus::Pending,
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Compute the next run time for a cron expression.
/// Normalizes 5-field cron to 6-field (prepends seconds=0) for the `cron` crate.
pub fn compute_next_run(cron_expr: &str) -> Option<f64> {
    let normalized = normalize_cron_expr(cron_expr);
    let schedule = normalized.parse::<cron::Schedule>().ok()?;
    let next = schedule.upcoming(chrono::Utc).next()?;
    Some(next.timestamp() as f64)
}

fn normalize_cron_expr(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_DDL).unwrap();
        conn
    }

    #[test]
    fn crud_tasks() {
        let conn = test_conn();
        let task = add_task(&conn, "Check email", TaskSource::User, TaskRecurrence::Once).unwrap();
        assert_eq!(task.title, "Check email");
        assert!(!task.completed);

        let fetched = get_task(&conn, &task.id).unwrap();
        assert_eq!(fetched.title, "Check email");

        let all = list_tasks(&conn, false).unwrap();
        assert_eq!(all.len(), 1);

        update_task(
            &conn,
            &task.id,
            &TaskPatch {
                title: Some("Check Gmail".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let updated = get_task(&conn, &task.id).unwrap();
        assert_eq!(updated.title, "Check Gmail");

        mark_task_completed(&conn, &task.id).unwrap();
        let done = get_task(&conn, &task.id).unwrap();
        assert!(done.completed);

        remove_task(&conn, &task.id).unwrap();
        assert!(get_task(&conn, &task.id).is_err());
    }

    #[test]
    fn due_tasks_filters_correctly() {
        let conn = test_conn();
        let now = now_secs();

        // Task with no next_run_at — should be due
        add_task(
            &conn,
            "No schedule",
            TaskSource::User,
            TaskRecurrence::Pending,
        )
        .unwrap();

        // Task with future next_run_at — should NOT be due
        let future_task =
            add_task(&conn, "Future task", TaskSource::User, TaskRecurrence::Once).unwrap();
        update_task_run_times(&conn, &future_task.id, now, Some(now + 3600.0)).unwrap();

        // Task with past next_run_at — should be due
        let past_task =
            add_task(&conn, "Past due", TaskSource::User, TaskRecurrence::Once).unwrap();
        update_task_run_times(&conn, &past_task.id, now - 7200.0, Some(now - 3600.0)).unwrap();

        let due = due_tasks(&conn, now).unwrap();
        assert_eq!(due.len(), 2); // "No schedule" + "Past due"
        assert!(due.iter().any(|t| t.title == "No schedule"));
        assert!(due.iter().any(|t| t.title == "Past due"));
        assert!(!due.iter().any(|t| t.title == "Future task"));
    }

    #[test]
    fn crud_log_entries() {
        let conn = test_conn();
        let task = add_task(&conn, "Test", TaskSource::User, TaskRecurrence::Once).unwrap();
        let now = now_secs();

        let entry = add_log_entry(
            &conn,
            &task.id,
            now,
            "act",
            Some("Did the thing"),
            Some(150),
        )
        .unwrap();
        assert_eq!(entry.decision, "act");

        let entries = list_log_entries(&conn, Some(&task.id), 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].result.as_deref(), Some("Did the thing"));

        let all_entries = list_log_entries(&conn, None, 10).unwrap();
        assert_eq!(all_entries.len(), 1);
    }

    #[test]
    fn crud_escalations() {
        let conn = test_conn();
        let task = add_task(&conn, "Test", TaskSource::User, TaskRecurrence::Once).unwrap();

        let esc = add_escalation(
            &conn,
            &task.id,
            None,
            "Deadline moved",
            "The API deadline was moved to tomorrow",
            &EscalationPriority::Critical,
        )
        .unwrap();
        assert_eq!(esc.status, EscalationStatus::Pending);

        let pending = list_escalations(&conn, Some(&EscalationStatus::Pending)).unwrap();
        assert_eq!(pending.len(), 1);

        assert_eq!(pending_escalation_count(&conn).unwrap(), 1);

        resolve_escalation(&conn, &esc.id, &EscalationStatus::Approved).unwrap();
        let resolved = get_escalation(&conn, &esc.id).unwrap();
        assert_eq!(resolved.status, EscalationStatus::Approved);
        assert!(resolved.resolved_at.is_some());

        assert_eq!(pending_escalation_count(&conn).unwrap(), 0);
    }

    #[test]
    fn seed_default_tasks_creates_system_tasks() {
        let conn = test_conn();

        let count = seed_default_tasks(&conn).unwrap();
        assert_eq!(count, DEFAULT_SYSTEM_TASKS.len());

        // Second seed should not duplicate
        let count2 = seed_default_tasks(&conn).unwrap();
        assert_eq!(count2, 0);

        let tasks = list_tasks(&conn, false).unwrap();
        assert_eq!(tasks.len(), DEFAULT_SYSTEM_TASKS.len());
        assert!(tasks.iter().all(|t| t.source == TaskSource::System));
    }

    #[test]
    fn recurrence_roundtrip() {
        assert_eq!(
            string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Once)),
            TaskRecurrence::Once
        );
        assert_eq!(
            string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Pending)),
            TaskRecurrence::Pending
        );
        assert_eq!(
            string_to_recurrence(&recurrence_to_string(&TaskRecurrence::Cron(
                "0 8 * * *".into()
            ))),
            TaskRecurrence::Cron("0 8 * * *".into())
        );
    }
}
