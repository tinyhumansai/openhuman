//! SQLite persistence for the subconscious engine.
//!
//! Follows the cron module's `with_connection` pattern: opens the database,
//! runs DDL on every connection, and provides pure functions.
//!
//! ## Init-failure noise suppression (TAURI-RUST-A)
//!
//! `with_connection` runs the schema DDL on every call. Transient
//! `SQLITE_BUSY` / `SQLITE_LOCKED` errors are handled by a per-connection
//! busy timeout (5 s) plus an application-level retry loop (3 retries,
//! 100 / 300 / 900 ms backoff).

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);
const OPEN_RETRY_ATTEMPTS: u32 = 3;
const OPEN_RETRY_BASE_MS: u64 = 100;

pub fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("subconscious").join("subconscious.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create subconscious dir: {}", parent.display()))?;
    }

    let conn = open_and_initialize_with_retry(&db_path)?;
    f(&conn)
}

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

fn open_and_initialize(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open subconscious DB: {}", db_path.display()))?;

    conn.busy_timeout(BUSY_TIMEOUT)
        .context("configure subconscious busy_timeout")?;

    // Choose a journal mode this filesystem can actually back *before* running
    // the table DDL. WAL is preferred for read/write concurrency, but it
    // eagerly opens an mmap-backed `-shm` shared-memory segment; on network
    // mounts, FUSE, and some sandboxed/synced macOS paths that mmap fails with
    // SQLITE_IOERR_SHMMAP (4618) or SQLITE_CANTOPEN (14). Previously the
    // `PRAGMA journal_mode = WAL` lived inside `SCHEMA_DDL`, so such a failure
    // aborted the whole schema-init batch and hung the Intelligence tab
    // (issue #3231 / TAURI-RUST-8WM). WAL is a performance optimization, not a
    // correctness requirement, so degrade to a rollback journal that needs no
    // shared memory rather than failing init.
    apply_journal_mode(&conn);

    conn.execute_batch(SCHEMA_DDL)
        .context("failed to run subconscious schema DDL")?;

    Ok(conn)
}

/// Set the SQLite journal mode for the subconscious DB, preferring WAL but
/// gracefully degrading to a rollback journal when the filesystem can't back
/// WAL's shared-memory segment.
///
/// `PRAGMA journal_mode = WAL` opens (and mmaps) the `-shm` file eagerly, so a
/// filesystem that can't support shared memory surfaces the failure here as
/// `SQLITE_IOERR_SHMMAP` / `SQLITE_CANTOPEN` rather than at first write. We try
/// WAL, and on any error — or if the engine silently refuses to switch (e.g.
/// in-memory DBs report `memory`) — fall back to `TRUNCATE`, which uses an
/// ordinary rollback journal and needs no shared memory. Failures are logged,
/// never propagated: a working rollback journal is strictly better than a
/// failed schema init, and the caller's DDL/queries work identically either
/// way.
///
/// Logs use a stable `db = "subconscious.db"` identifier rather than the
/// absolute path — the workspace path embeds the user's home dir / username, so
/// emitting it would leak PII into the breadcrumb stream.
fn apply_journal_mode(conn: &Connection) {
    match query_journal_mode(conn, "WAL") {
        Ok(mode) if mode.eq_ignore_ascii_case("wal") => return,
        Ok(mode) => {
            tracing::warn!(
                target: "openhuman::subconscious::store",
                db = "subconscious.db",
                requested = "wal",
                effective = %mode,
                "[subconscious::store] WAL journal mode not honoured; falling back to TRUNCATE"
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "openhuman::subconscious::store",
                db = "subconscious.db",
                error = %format!("{e}"),
                "[subconscious::store] WAL journal mode failed (filesystem can't back -shm); falling back to TRUNCATE"
            );
        }
    }

    if let Err(e) = query_journal_mode(conn, "TRUNCATE") {
        tracing::warn!(
            target: "openhuman::subconscious::store",
            db = "subconscious.db",
            error = %format!("{e}"),
            "[subconscious::store] TRUNCATE journal fallback also failed; continuing with engine default"
        );
    }
}

/// Run `PRAGMA journal_mode = <mode>` and return the resulting mode string.
///
/// `mode` is a fixed internal SQLite keyword (`WAL`, `TRUNCATE`, …), never user
/// input. PRAGMA statements don't accept bound parameters, so inlining the
/// keyword in the statement text is the only option and is safe here.
fn query_journal_mode(conn: &Connection, mode: &str) -> rusqlite::Result<String> {
    conn.query_row(&format!("PRAGMA journal_mode = {mode}"), [], |row| {
        row.get::<_, String>(0)
    })
}

fn is_sqlite_busy(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        return matches!(
            sqlite_err.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        );
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database is locked") || msg.contains("database table is locked")
}

// NOTE: `PRAGMA journal_mode` is intentionally *not* part of this batch — it is
// applied separately via `apply_journal_mode` so a filesystem that can't back
// WAL's `-shm` segment degrades to a rollback journal instead of aborting the
// whole schema init (issue #3231). Keep journal-mode selection out of here.
const SCHEMA_DDL: &str = "
    PRAGMA foreign_keys = ON;

    -- Legacy tables retained for backward compatibility with existing DBs.
    -- No longer written to or read from.
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
    CREATE TABLE IF NOT EXISTS subconscious_log (
        id          TEXT PRIMARY KEY,
        task_id     TEXT NOT NULL,
        tick_at     REAL NOT NULL,
        decision    TEXT NOT NULL,
        result      TEXT,
        duration_ms INTEGER,
        created_at  REAL NOT NULL
    );
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

    CREATE TABLE IF NOT EXISTS subconscious_reflections (
        id              TEXT PRIMARY KEY,
        kind            TEXT NOT NULL,
        body            TEXT NOT NULL,
        proposed_action TEXT,
        source_refs     TEXT NOT NULL DEFAULT '[]',
        source_chunks   TEXT NOT NULL DEFAULT '[]',
        created_at      REAL NOT NULL,
        acted_on_at     REAL,
        dismissed_at    REAL,
        thread_id       TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_reflections_created
        ON subconscious_reflections(created_at DESC);

    CREATE TABLE IF NOT EXISTS subconscious_hotness_snapshots (
        entity_id       TEXT PRIMARY KEY,
        score           REAL NOT NULL,
        captured_at     REAL NOT NULL
    );

    CREATE TABLE IF NOT EXISTS subconscious_state (
        key   TEXT PRIMARY KEY,
        value REAL NOT NULL
    );

    -- Text-valued engine state (the REAL-typed `subconscious_state` above can't
    -- hold ids). Used for the memory_diff baseline checkpoint id the structured
    -- tick diffs against.
    CREATE TABLE IF NOT EXISTS subconscious_state_text (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
";

#[cfg(test)]
pub(crate) const SCHEMA_DDL_FOR_TESTS: &str = SCHEMA_DDL;

// ── Engine state KV ──────────────────────────────────────────────────────────

const STATE_KEY_LAST_TICK_AT: &str = "last_tick_at";
const STATE_KEY_BASELINE_CHECKPOINT_ID: &str = "baseline_checkpoint_id";

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

pub fn set_last_tick_at(conn: &Connection, value: f64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO subconscious_state (key, value) VALUES (?1, ?2)",
        rusqlite::params![STATE_KEY_LAST_TICK_AT, value],
    )?;
    Ok(())
}

/// The memory_diff checkpoint the structured tick diffs against — i.e. the
/// snapshot of "the agent's world" captured at the end of the previous tick.
/// `None` until the first tick establishes a baseline.
pub fn get_baseline_checkpoint_id(conn: &Connection) -> Result<Option<String>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM subconscious_state_text WHERE key = ?1",
            [STATE_KEY_BASELINE_CHECKPOINT_ID],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value)
}

pub fn set_baseline_checkpoint_id(conn: &Connection, checkpoint_id: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO subconscious_state_text (key, value) VALUES (?1, ?2)",
        rusqlite::params![STATE_KEY_BASELINE_CHECKPOINT_ID, checkpoint_id],
    )?;
    Ok(())
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
