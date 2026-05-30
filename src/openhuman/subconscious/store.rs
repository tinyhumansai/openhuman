//! SQLite persistence for subconscious tasks, execution log, and escalations.
//!
//! Follows the cron module's `with_connection` pattern: opens the database,
//! runs DDL on every connection, and provides pure functions.
//!
//! ## Init-failure noise suppression (TAURI-RUST-A)
//!
//! `with_connection` runs the schema DDL on every call. Transient
//! `SQLITE_BUSY` / `SQLITE_LOCKED` errors (e.g. concurrent in-process RPC
//! calls, antivirus hold, network-drive WAL rejection) are handled by a
//! per-connection busy timeout (5 s) plus an application-level retry loop
//! (3 retries, 100 / 300 / 900 ms backoff). Only `DatabaseBusy` /
//! `DatabaseLocked` errors are retried — schema or corruption errors fail
//! through immediately so Sentry captures a real root cause rather than a
//! transient noise event.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

use super::types::{
    Escalation, EscalationPriority, EscalationStatus, SubconsciousLogEntry, SubconsciousTask,
    TaskPatch, TaskRecurrence, TaskSource,
};

/// Per-connection busy handler window. Tracks the value used by the cron
/// module + other domain stores (`memory_store::unified::init`, 15s) and
/// the higher-throughput whatsapp/memory_queue path (5s). 5 s is enough
/// for the subconscious tick — RPC handlers are user-driven (status
/// polling at 3 s, manual triggers) and we'd rather fail fast than block
/// the UI thread for the full 15 s of contention.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Maximum number of application-level retries after rusqlite's busy
/// handler is exhausted. The first attempt is "attempt 0" — total
/// attempts = `OPEN_RETRY_ATTEMPTS` + 1.
const OPEN_RETRY_ATTEMPTS: u32 = 3;

/// Base backoff for application-level retries; per-attempt sleep is
/// `BASE * 3^attempt` so the schedule is `100 ms / 300 ms / 900 ms`
/// totalling ≤ 1.3 s before the final attempt fails through.
const OPEN_RETRY_BASE_MS: u64 = 100;

/// Open the subconscious database and run schema migrations.
///
/// Three layers of defence against transient `SQLITE_BUSY` / `SQLITE_LOCKED`
/// at the open / DDL boundary, motivated by Sentry TAURI-RUST-A
/// (cross-platform, ~1.3k events / 24 h, RPC paths `subconscious_tasks_list`
/// and `subconscious_status`):
///
/// 1. **Per-connection busy timeout** (`BUSY_TIMEOUT`, 5 s): SQLite's
///    default is `0` — first lock contention returns `SQLITE_BUSY`
///    immediately. The subconscious domain serialises several RPCs
///    (status poll every 3 s, tasks-list on Intelligence page, manual
///    trigger), each opening its own connection; without a timeout the
///    first concurrent open races and one returns `SQLITE_BUSY` mid-DDL.
/// 2. **Application-level retry** (3 attempts, exponential backoff
///    100 / 300 / 900 ms): catches the residual case where the busy
///    handler is exhausted (long-running external write txn, AV scan
///    holding the file). Mirrors `whatsapp_data::sqlite_retry` /
///    `memory_queue::worker::is_sqlite_busy`.
/// 3. **Retry classifier** (`is_sqlite_busy`): only retries
///    `DatabaseBusy` / `DatabaseLocked`. Schema / syntax / corruption
///    errors are real bugs or unrecoverable file-state failures —
///    retrying just delays the report.
pub fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("subconscious").join("subconscious.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create subconscious dir: {}", parent.display()))?;
    }
    let conn = open_and_initialize_with_retry(&db_path)?;
    f(&conn)
}

/// Open the SQLite file, set `busy_timeout`, run `SCHEMA_DDL`, and apply
/// the idempotent reflection-store migrations — retrying the whole
/// sequence on `SQLITE_BUSY` / `SQLITE_LOCKED`. Split out so the retry
/// loop has a single failure surface to classify and the happy path
/// stays linear.
fn open_and_initialize_with_retry(db_path: &Path) -> Result<Connection> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=OPEN_RETRY_ATTEMPTS {
        match open_and_initialize(db_path) {
            Ok(conn) => {
                if attempt > 0 {
                    tracing::debug!(
                        target: "openhuman::subconscious::store",
                        attempt = attempt,
                        db_path = %db_path.display(),
                        "[subconscious::store] open/DDL succeeded after {attempt} busy retries"
                    );
                }
                return Ok(conn);
            }
            Err(e) => {
                if !is_sqlite_busy(&e) || attempt == OPEN_RETRY_ATTEMPTS {
                    last_err = Some(e);
                    break;
                }
                let sleep_ms = OPEN_RETRY_BASE_MS
                    .saturating_mul(3u64.saturating_pow(attempt))
                    .min(900);
                tracing::warn!(
                    target: "openhuman::subconscious::store",
                    attempt = attempt + 1,
                    max_attempts = OPEN_RETRY_ATTEMPTS + 1,
                    sleep_ms = sleep_ms,
                    error = %format!("{e:#}"),
                    "[subconscious::store] SQLite busy/locked on open or DDL; retrying"
                );
                std::thread::sleep(Duration::from_millis(sleep_ms));
                last_err = Some(e);
            }
        }
    }

    Err(last_err.expect("OPEN_RETRY_ATTEMPTS >= 0 ensures at least one attempt"))
}

/// Single-shot open + DDL + migrations. Each invocation returns an
/// owned `Connection`; on failure the partially-initialised connection
/// is dropped before the caller retries.
fn open_and_initialize(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open subconscious DB: {}", db_path.display()))?;

    // Set busy_timeout BEFORE running DDL — the very first PRAGMA / CREATE
    // TABLE in SCHEMA_DDL can race with another in-process connection
    // (subconscious RPCs each call `with_connection` independently), and
    // SQLite's default busy_timeout is 0.
    conn.busy_timeout(BUSY_TIMEOUT)
        .context("configure subconscious busy_timeout")?;

    conn.execute_batch(SCHEMA_DDL)
        .context("failed to run subconscious schema DDL")?;

    // Drop the legacy `disposition` / `surfaced_at` columns + their index
    // from previously-migrated DBs. Idempotent — fresh installs and
    // already-migrated DBs no-op via swallowed errors.
    super::reflection_store::migrate_drop_legacy_columns(&conn);

    // Add the `source_chunks` JSON column to previously-migrated DBs.
    // Idempotent (duplicate-column errors swallowed).
    super::reflection_store::migrate_add_source_chunks_column(&conn);

    Ok(conn)
}

/// Returns true when `err` is transient SQLite contention worth retrying
/// (`SQLITE_BUSY` / `SQLITE_LOCKED`). Schema / syntax / corruption errors
/// are NOT retried — the retry would just delay the same failure.
///
/// Modelled on [`crate::openhuman::memory_queue::worker::is_sqlite_busy`]
/// and [`crate::openhuman::whatsapp_data::sqlite_retry::is_sqlite_busy`];
/// kept private to the subconscious store so the retry policy can evolve
/// independently of those sibling domains.
fn is_sqlite_busy(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        return matches!(
            sqlite_err.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        );
    }
    // Fallback for errors wrapped under `.context(...)` layers — the
    // rusqlite root may sit a few levels deep after `with_context`
    // wraps the open / DDL failure. anyhow's alternate Display joins
    // every cause with ": " so the SQLite-rendered phrase remains
    // searchable.
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database is locked") || msg.contains("database table is locked")
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

    -- #623: reflection layer (proactive subconscious). Mirrored in
    -- `super::reflection_store::REFLECTION_SCHEMA_DDL` for the unit
    -- tests there. Legacy `disposition` / `surfaced_at` columns +
    -- their index were removed when the auto-post-into-thread flow
    -- was dropped — `migrate_drop_legacy_columns` cleans them off
    -- previously-migrated DBs. The `source_chunks` JSON column was
    -- added later for the memory-context snapshot feature —
    -- `migrate_add_source_chunks_column` backfills previously-migrated
    -- DBs that pre-date it.
    CREATE TABLE IF NOT EXISTS subconscious_reflections (
        id              TEXT PRIMARY KEY,
        kind            TEXT NOT NULL,
        body            TEXT NOT NULL,
        proposed_action TEXT,
        source_refs     TEXT NOT NULL DEFAULT '[]',
        source_chunks   TEXT NOT NULL DEFAULT '[]',
        created_at      REAL NOT NULL,
        acted_on_at     REAL,
        dismissed_at    REAL
    );
    CREATE INDEX IF NOT EXISTS idx_reflections_created
        ON subconscious_reflections(created_at DESC);

    CREATE TABLE IF NOT EXISTS subconscious_hotness_snapshots (
        entity_id       TEXT PRIMARY KEY,
        score           REAL NOT NULL,
        captured_at     REAL NOT NULL
    );

    -- Tiny KV table for engine-local state that needs to survive
    -- process restarts. Currently holds:
    --   * `last_tick_at`  — unix-seconds float of the most recent
    --                       successful tick. Used by the situation-
    --                       report sections (`summaries`, `query_window`,
    --                       `digest`) as a `WHERE sealed_at_ms > ?` cutoff
    --                       so the LLM only sees memory-tree rows that
    --                       have appeared since it last looked. Without
    --                       persistence the cutoff resets to 0 on every
    --                       restart, the LLM keeps reading the same
    --                       summaries, and `persist_and_surface_reflections`
    --                       (which has no insert-time dedupe) accumulates
    --                       near-duplicate reflections about the same
    --                       chunks (#623).
    CREATE TABLE IF NOT EXISTS subconscious_state (
        key   TEXT PRIMARY KEY,
        value REAL NOT NULL
    );
";

/// Test-only re-export of [`SCHEMA_DDL`] for unit tests in sibling
/// modules (e.g. `reflection_store_tests`) that need to spin up an
/// in-memory connection with the full schema.
#[cfg(test)]
pub(crate) const SCHEMA_DDL_FOR_TESTS: &str = SCHEMA_DDL;

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

// ── Engine state KV ──────────────────────────────────────────────────────────

/// SQLite key for the most recent successful tick, in unix seconds.
/// Loaded by [`SubconsciousEngine::from_heartbeat_config`] on init and
/// updated after every successful tick. See `subconscious_state` table
/// docstring in [`SCHEMA_DDL`] for the dedupe rationale.
const STATE_KEY_LAST_TICK_AT: &str = "last_tick_at";

/// Read the persisted `last_tick_at` from `subconscious_state`. Returns
/// `0.0` when the row is absent (cold start or fresh workspace) so the
/// caller can treat "never ticked" identically to "first run".
pub fn get_last_tick_at(conn: &Connection) -> Result<f64> {
    let value: Option<f64> = conn
        .query_row(
            "SELECT value FROM subconscious_state WHERE key = ?1",
            [STATE_KEY_LAST_TICK_AT],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.unwrap_or(0.0))
}

/// Persist `last_tick_at` so the next process restart picks up where
/// this run left off. Upsert via `INSERT OR REPLACE` — the table is one
/// row per key, so collisions are the expected case.
pub fn set_last_tick_at(conn: &Connection, value: f64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO subconscious_state (key, value) VALUES (?1, ?2)",
        rusqlite::params![STATE_KEY_LAST_TICK_AT, value],
    )?;
    Ok(())
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
#[path = "store_tests.rs"]
mod tests;
