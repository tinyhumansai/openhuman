//! One-shot import of the legacy `sources.db` tables into the document layout
//! (`store_documents.rs`): `task_sources` becomes one `task_sources` document
//! per source (the store's own `to_doc`) and `ingested_tasks` one
//! `ingested_tasks` document per dedup-ledger entry, with `ingested_ms`
//! derived from `ingested_at`. A source row that no longer decodes fails the
//! import, which is retried on the next boot. The legacy file is only ever
//! opened here, by the storage importer, on a scratch copy of it.

use crate::storage::rusqlite;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::DateTime;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::storage_schema::{LEGACY_INGESTED, LEGACY_SOURCES};
use super::store_documents::to_doc;
use crate::config::Config;
use crate::storage::import::{table_exists, ImportDoc};

/// Reads every legacy row, after bringing an old schema forward (on the
/// importer's scratch copy of the file).
pub(crate) fn read(config: &Config) -> Result<Vec<ImportDoc>> {
    with_connection(config, read_rows)
}

fn read_rows(conn: &Connection) -> Result<Vec<ImportDoc>> {
    let mut docs = Vec::new();
    if table_exists(conn, "task_sources")? {
        let mut stmt = conn
            .prepare(&format!(
                "{} ORDER BY created_at ASC, id ASC",
                SELECT_SOURCE_COLUMNS
            ))
            .context("[task_sources::import] prepare task_sources")?;
        let rows = stmt
            .query_map([], map_source_row)
            .context("[task_sources::import] query task_sources")?;
        for row in rows {
            // A row that does not decode fails the import, so the legacy
            // table is not retired with a source missing from the new one.
            let source = row.context("[task_sources::import] decode a task_sources row")?;
            docs.push(ImportDoc {
                collection: LEGACY_SOURCES,
                id: source.id.clone(),
                doc: to_doc(&source)?,
            });
        }
    }
    if table_exists(conn, "ingested_tasks")? {
        let mut stmt = conn
            .prepare(
                "SELECT source_id, external_id, content_hash, title, payload, ingested_at
                 FROM ingested_tasks ORDER BY rowid",
            )
            .context("[task_sources::import] prepare ingested_tasks")?;
        let rows = stmt
            .query_map([], |row| {
                let source_id: String = row.get(0)?;
                let external_id: String = row.get(1)?;
                let ingested_at: String = row.get(5)?;
                let mut doc = Map::new();
                doc.insert("source_id".into(), json!(source_id));
                doc.insert("external_id".into(), json!(external_id));
                doc.insert("content_hash".into(), json!(row.get::<_, String>(2)?));
                if let Some(title) = row.get::<_, Option<String>>(3)? {
                    doc.insert("title".into(), json!(title));
                }
                if let Some(payload) = row.get::<_, Option<String>>(4)? {
                    doc.insert("payload".into(), json!(payload));
                }
                let ingested = DateTime::parse_from_rfc3339(&ingested_at).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                doc.insert("ingested_ms".into(), json!(ingested.timestamp_millis()));
                doc.insert("ingested_at".into(), json!(ingested_at));
                Ok(ImportDoc {
                    collection: LEGACY_INGESTED,
                    id: format!("{}:{source_id}/{external_id}", source_id.len()),
                    doc: Value::Object(doc),
                })
            })
            .context("[task_sources::import] query ingested_tasks")?;
        for row in rows {
            docs.push(row.context("[task_sources::import] decode an ingested_tasks row")?);
        }
    }
    Ok(docs)
}

// ---------------------------------------------------------------------------
// The legacy database
// ---------------------------------------------------------------------------

#[path = "store_import_rows.rs"]
mod store_rows;
use store_rows::{map_source_row, SELECT_SOURCE_COLUMNS};

/// Tracks which task_sources database files have already had their schema DDL
/// (the `CREATE TABLE`/`CREATE INDEX` batch plus the `add_column_if_missing`
/// migration probes) run against them in this process. `with_connection`
/// deliberately keeps opening a fresh `rusqlite::Connection` per call —
/// `Connection` is `!Sync`, so caching a single shared one would need a
/// process-wide mutex that serializes every caller. What repeats needlessly on
/// every open is the DDL batch itself (2 `CREATE TABLE` + 1 `CREATE INDEX`) plus
/// the 2 `PRAGMA table_info(...)` migration scans — paid before every store op,
/// and the periodic-poll fetch loop hits three of them per task (`is_ingested`,
/// `was_ingested`, `mark_ingested`). Gating just that batch behind a per-path
/// "already initialized" set keeps it to one execution per process per database
/// file while every call still gets its own connection.
///
/// This mirrors `flows::store`'s `INITIALIZED_SCHEMAS` (R-m8) and the companion
/// `cron::store` gating — the `task_sources` store was itself derived from the
/// `cron` `with_connection` idiom.
///
/// Keyed by path rather than a single flag: tests each open an independent
/// per-`TempDir` workspace within the same test binary, and a bare
/// `OnceLock<()>` would silently skip schema creation for every database path
/// after the first test to run in the process.
static INITIALIZED_SCHEMAS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// On-disk schema version stamped into `PRAGMA user_version` by [`init_schema`]
/// and checked by [`ensure_schema_initialized`] on a cache hit. **Bump this
/// whenever a table or `add_column_if_missing` migration is added to
/// [`init_schema`]** so a database replaced at runtime with an older/partial
/// schema is re-migrated rather than trusted.
const TASK_SOURCES_SCHEMA_VERSION: i64 = 1;

/// Ensures the schema on `conn` (a connection to `db_path`) is present and fully
/// migrated, running the DDL + migrations at most once per process per path.
///
/// **The on-disk `PRAGMA user_version` is the authority, and the fast path is
/// lock-free.** [`init_schema`] stamps [`TASK_SOURCES_SCHEMA_VERSION`] only after
/// a full, successful migration, so a connection whose `user_version >=
/// TASK_SOURCES_SCHEMA_VERSION` is known-good and returns without touching any
/// process-global state — the common case (an already-initialized store) never
/// acquires the initialization mutex. Reading `user_version` is a single
/// database-header read, far cheaper than the DDL batch + metadata scans it
/// replaces. Treating newer on-disk versions as already-initialized avoids
/// needlessly downgrading the stamp on a database created by a newer binary
/// (mirroring `security/approval/store.rs`'s idiom).
///
/// **Initialization is serialized and atomic per process.** Only a version
/// *mismatch* takes the [`INITIALIZED_SCHEMAS`] lock, and `user_version` is
/// re-read under it, so two first callers racing on the same fresh path run the
/// DDL exactly once — the loser observes the winner's stamp on the recheck and
/// returns (CodeRabbit: initialization must be atomic per path). Independent db
/// paths contend only during that rare init window, never on the hot path.
///
/// **Trust, but verify.** Before this gating existed the DDL ran on every
/// `with_connection` call, so a database deleted or replaced at runtime (a
/// workspace reset, a manual deletion, a disk-recovery restore) self-healed on
/// the next call. The version gate restores that: a `user_version` lower than
/// the expected version — 0 for a fresh file, or an older/partial schema
/// swapped in under a live process (missing `ingested_tasks` or a migrated
/// column such as `card_id`) — falls through to the
/// idempotent [`init_schema`] and is re-migrated rather than trusted and later
/// failing on the incomplete schema (CodeRabbit / Codex review on #5709). An
/// on-disk version *higher* than expected is treated as already-initialized
/// (forward-compatible: an older binary opening a newer database does not
/// downgrade the stamp).
///
/// [`INITIALIZED_SCHEMAS`] no longer gates the DDL — the on-disk version does —
/// and is kept purely as a **diagnostic marker**: a path present in the set
/// whose on-disk version no longer matches was initialized by *this* process and
/// has since been deleted or replaced, which is worth a warning; a path absent
/// from the set is an ordinary first-ever init and stays silent.
fn ensure_schema_initialized(conn: &mut Connection, db_path: &Path) -> Result<()> {
    let is_current = || -> bool {
        let version = conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        // Treat any on-disk version >= expected as already initialized.
        // This avoids downgrading the stamp when an older binary opens a
        // database created by a newer version (the repo's own idiom,
        // mirroring `security/approval/store.rs`).
        version >= TASK_SOURCES_SCHEMA_VERSION
    };

    // Lock-free fast path: an already-migrated database carries
    // TASK_SOURCES_SCHEMA_VERSION in its header, so the common case never
    // acquires the process-global initialization mutex.
    if is_current() {
        return Ok(());
    }

    // Mismatch ⇒ (re-)initialization is required. Serialize it so two first
    // callers for the same path cannot both run the DDL, and re-read the version
    // under the lock — another thread may have migrated between the lock-free
    // read above and acquiring the guard.
    let initialized = INITIALIZED_SCHEMAS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = initialized
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if is_current() {
        return Ok(());
    }

    // Diagnostic only (see the doc comment): a cached path whose on-disk schema
    // no longer matches was deleted or replaced under a live process — a
    // first-ever init leaves the path absent and stays silent.
    if guard.contains(db_path) {
        tracing::warn!(
            target: "task_sources",
            db = %db_path.display(),
            expected_version = TASK_SOURCES_SCHEMA_VERSION,
            "[task_sources:store] a database this process already initialized no longer matches the expected schema version (deleted or replaced at runtime?) — re-running schema init"
        );
    }

    init_schema(conn)?;
    guard.insert(db_path.to_path_buf());
    Ok(())
}

/// The actual schema DDL + post-hoc column migrations, split out of
/// `with_connection` so [`ensure_schema_initialized`] can gate it. Does **not**
/// carry the open-time pragmas (`busy_timeout`, `journal_mode = WAL`,
/// `foreign_keys = ON`): those are (re)applied on every open in
/// `with_connection`, outside this gate.
///
/// **Wrapped in an immediate transaction** so the schema version stamp is
/// atomic with the DDL. The transaction is begun *before* the DDL runs, and
/// `user_version` is re-read under it — if another process has already
/// completed initialization between the caller's lock-free check and the
/// `BEGIN IMMEDIATE`, we bail early. This prevents stamping a stale version
/// on a database a concurrent process already migrated (CodeRabbit #5709).
fn init_schema(conn: &mut Connection) -> Result<()> {
    // Immediate transaction: blocks other writers (and readers in WAL mode
    // with IMMEDIATE) so no two processes can migrate the same database
    // concurrently. `BEGIN IMMEDIATE` succeeds even in WAL mode.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .context("Failed to begin transaction for task_sources schema init")?;

    // Re-check the version under the transaction — another process may have
    // completed initialization between the lock-free check in
    // `ensure_schema_initialized` and this `BEGIN IMMEDIATE`.
    let current_version: i64 = tx
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);
    if current_version >= TASK_SOURCES_SCHEMA_VERSION {
        // Another process got there first. Roll back the empty transaction
        // and return — the schema is already up to date.
        return Ok(());
    }

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_sources (
            id                  TEXT PRIMARY KEY,
            provider            TEXT NOT NULL,
            connection_id       TEXT,
            name                TEXT,
            enabled             INTEGER NOT NULL DEFAULT 1,
            filter              TEXT NOT NULL,
            interval_secs       INTEGER NOT NULL,
            target              TEXT NOT NULL,
            max_tasks_per_fetch INTEGER NOT NULL,
            created_at          TEXT NOT NULL,
            last_fetch_at       TEXT,
            last_status         TEXT
         );
         CREATE TABLE IF NOT EXISTS ingested_tasks (
            source_id    TEXT NOT NULL,
            external_id  TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            title        TEXT,
            payload      TEXT,
            ingested_at  TEXT NOT NULL,
            card_id      TEXT,
            PRIMARY KEY (source_id, external_id),
            FOREIGN KEY (source_id) REFERENCES task_sources(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_ingested_source ON ingested_tasks(source_id, ingested_at);",
    )
    .context("Failed to initialize task_sources schema")?;

    // Additive migration: add card_id to existing databases that pre-date
    // this column. Tolerate "duplicate column" in case of a concurrent open.
    add_column_if_missing(&tx, "ingested_tasks", "card_id", "TEXT")?;

    // Stamp the schema version last, so [`ensure_schema_initialized`] only
    // trusts a cache hit whose on-disk schema is fully migrated. Bump
    // TASK_SOURCES_SCHEMA_VERSION whenever a table or migration is added above.
    tx.pragma_update(None, "user_version", TASK_SOURCES_SCHEMA_VERSION)
        .context("Failed to stamp task_sources schema version")?;

    tx.commit()
        .context("Failed to commit task_sources schema init transaction")?;

    Ok(())
}

pub(crate) fn db_path(config: &Config) -> PathBuf {
    config.workspace_dir.join("task_sources").join("sources.db")
}

pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = db_path(config);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create task_sources directory: {}",
                parent.display()
            )
        })?;
    }
    let mut conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open task_sources DB: {}", db_path.display()))?;
    // Open-time pragmas, reapplied on every open regardless of the schema-init
    // cache below (they configure the connection, not the schema). Overlapping
    // periodic-poll + UI writes can otherwise hit "database is locked"; WAL + a
    // busy timeout match the other stores, and `foreign_keys` is required on
    // every connection for the `ingested_tasks` ON DELETE CASCADE FK to be
    // enforced. `foreign_keys` was moved out of the gated DDL batch to here so
    // it still runs on every open once the batch stops doing so. (`journal_mode
    // = WAL` is persisted in the db file and need only run once, but is kept
    // here with the other open-time pragmas to keep the connection setup
    // byte-identical; it is idempotent per open.)
    conn.busy_timeout(Duration::from_secs(5))
        .context("Failed to configure task_sources DB busy timeout")?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("Failed to enable WAL for task_sources DB")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to set task_sources DB connection pragmas")?;

    ensure_schema_initialized(&mut conn, &db_path)?;

    f(&conn)
}

/// Add a column to a table only when it is absent. Mirrors the pattern used
/// in `cron/store.rs` to keep migrations idempotent across DB versions.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    sql_type: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(1)?;
        if col_name == column {
            return Ok(()); // already present
        }
    }
    drop(rows);
    drop(stmt);
    match conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {sql_type}"),
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            tracing::debug!(
                "[task_sources:store] column {table}.{column} already exists (concurrent migration)"
            );
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("Failed to add {table}.{column}")),
    }
}

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "store_import_tests.rs"]
mod tests;
