use anyhow::{Context, Result};
use parking_lot::Mutex as PMutex;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::openhuman::config::Config;

use super::{
    add_column_if_missing, migrate_legacy_embeddings_to_sidecar, DB_DIR, DB_FILE, SCHEMA,
    SQLITE_BUSY_TIMEOUT,
};

// ── Schema-apply instrumentation (test-only) ─────────────────────────────────
//
// Per-path counter of how many times `apply_schema` ran for each DB path,
// gated behind `cfg(test)` so the production binary carries no overhead.
// Used by the concurrent-init regression test to assert "exactly once per
// path" across racing workers; it survives even when the connection cache
// is cleared between tests because tests use distinct tempdirs.
#[cfg(test)]
static SCHEMA_APPLY_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();

fn record_schema_apply(_path: &Path) {
    #[cfg(test)]
    {
        let counts = SCHEMA_APPLY_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = counts
            .lock()
            .expect("memory_tree schema apply count mutex poisoned");
        *guard.entry(_path.to_path_buf()).or_insert(0) += 1;
    }
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn schema_apply_count_for_path_for_tests(path: &Path) -> usize {
    SCHEMA_APPLY_COUNTS
        .get()
        .and_then(|m| {
            m.lock()
                .ok()
                .map(|guard| guard.get(path).copied().unwrap_or(0))
        })
        .unwrap_or(0)
}

// SQLite extended result codes that fire during cold-start WAL/SHM bootstrap
// races. NOTE on values: extended codes are `SQLITE_IOERR (10) | (sub << 8)`.
// 4874 is `IOERR_SHMSIZE` (sub 19), NOT `SHMMAP` — the real `SHMMAP` is 5386
// (sub 21) and the "open a new shared-memory segment" failure is `SHMOPEN`
// 4618 (sub 18), which is what surfaced on macOS. The whole `-shm` family is
// listed so the classifiers don't miss any of them.
/// `CANTOPEN` — racing the lockfile/WAL creation done by another connection.
const SQLITE_CANTOPEN: i32 = 14;
/// `IOERR_TRUNCATE` — the WAL/db is being truncated during bootstrap.
const SQLITE_IOERR_TRUNCATE: i32 = 1546;
/// `IOERR_SHMOPEN` — opening a new `-shm` shared-memory segment failed (the
/// macOS cold-start failure, e.g. Sentry TAURI-RUST-X1).
const SQLITE_IOERR_SHMOPEN: i32 = 4618;
/// `IOERR_SHMSIZE` — the `-shm` file is being resized during bootstrap.
const SQLITE_IOERR_SHMSIZE: i32 = 4874;
/// `IOERR_SHMMAP` — mapping a page of the `-shm` wal-index failed.
const SQLITE_IOERR_SHMMAP: i32 = 5386;
/// `IOERR_IN_PAGE` — an mmap-page I/O fault, also seen under WAL cold-start.
const SQLITE_IOERR_IN_PAGE: i32 = 8714;

/// True if `err` (or anything in its cause chain) is one of the SQLite codes
/// that fire during cold-start WAL/SHM bootstrap races: `CANTOPEN`,
/// `IOERR_TRUNCATE`, the `-shm` family (`SHMOPEN` / `SHMSIZE` / `SHMMAP`), and
/// `IOERR_IN_PAGE`.
pub(crate) fn is_transient_cold_start(err: &anyhow::Error) -> bool {
    fn is_transient_sqlite(e: &(dyn std::error::Error + 'static)) -> bool {
        if let Some(rusqlite::Error::SqliteFailure(ffi, _)) = e.downcast_ref::<rusqlite::Error>() {
            return matches!(
                ffi.extended_code,
                SQLITE_CANTOPEN
                    | SQLITE_IOERR_TRUNCATE
                    | SQLITE_IOERR_SHMOPEN
                    | SQLITE_IOERR_SHMSIZE
                    | SQLITE_IOERR_SHMMAP
                    | SQLITE_IOERR_IN_PAGE
            );
        }
        false
    }
    if is_transient_sqlite(err.root_cause()) {
        return true;
    }
    let mut src: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(cur) = src {
        if is_transient_sqlite(cur) {
            return true;
        }
        src = cur.source();
    }
    false
}

// ── Connection cache (#2206) ─────────────────────────────────────────────────

/// How many consecutive init failures before the circuit breaker trips.
pub(crate) const CB_THRESHOLD: u32 = 3;
/// How long the circuit breaker holds the DB closed after tripping.
pub(crate) const CB_COOLDOWN: Duration = Duration::from_secs(30);

/// Per-path circuit breaker: after [`CB_THRESHOLD`] consecutive init failures
/// the breaker trips and `get_or_init_connection` returns an error immediately
/// until [`CB_COOLDOWN`] elapses. On the first success it resets to zero.
struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    tripped: AtomicBool,
    last_trip: PMutex<Option<Instant>>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            tripped: AtomicBool::new(false),
            last_trip: PMutex::new(None),
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.tripped.store(false, Ordering::Relaxed);
        *self.last_trip.lock() = None;
    }

    /// Records one more failure. Returns `true` if this call just tripped the
    /// breaker (i.e. the threshold was crossed right now).
    fn record_failure(&self) -> bool {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        let count = prev + 1;
        if count >= CB_THRESHOLD && !self.tripped.swap(true, Ordering::Relaxed) {
            *self.last_trip.lock() = Some(Instant::now());
            return true;
        }
        // Re-arm the cooldown on each subsequent failure while already tripped
        // so that a failed post-cooldown retry restarts the 30 s window instead
        // of leaving the stale timestamp in place (which would let `is_open`
        // return false immediately and allow unlimited retries).
        if self.tripped.load(Ordering::Relaxed) {
            *self.last_trip.lock() = Some(Instant::now());
        }
        false
    }

    /// Returns `true` when the breaker is open AND the cooldown has not yet
    /// elapsed. Returns `false` (allowing a retry) once the cooldown passes.
    fn is_open(&self) -> bool {
        if !self.tripped.load(Ordering::Relaxed) {
            return false;
        }
        let guard = self.last_trip.lock();
        match *guard {
            Some(t) if t.elapsed() < CB_COOLDOWN => true,
            _ => false,
        }
    }
}

/// Process-level cache — two separate maps so that a failing init cannot
/// accidentally serve a dummy connection on the fast path.
///
/// `connections`: only fully-initialised (schema + migrations run) entries.
/// `breakers`:    persists across failed init attempts so the circuit breaker
///                survives even when `connections` has no entry for that path.
struct ConnectionCache {
    connections: PMutex<HashMap<PathBuf, Arc<PMutex<Connection>>>>,
    breakers: PMutex<HashMap<PathBuf, Arc<CircuitBreaker>>>,
    /// Per-path mutex held across the slow-path init so concurrent
    /// workers racing into `with_connection` on a cold DB serialise on
    /// the WAL+SHM bootstrap. Without this, N threads see "no cached
    /// connection" simultaneously and all run `apply_schema`, which is
    /// idempotent but reopens the cold-start race window
    /// (OPENHUMAN-TAURI-HH / -ZM / -MB).
    init_locks: PMutex<HashMap<PathBuf, Arc<PMutex<()>>>>,
}

static CONN_CACHE: OnceLock<ConnectionCache> = OnceLock::new();

fn conn_cache() -> &'static ConnectionCache {
    CONN_CACHE.get_or_init(|| ConnectionCache {
        connections: PMutex::new(HashMap::new()),
        breakers: PMutex::new(HashMap::new()),
        init_locks: PMutex::new(HashMap::new()),
    })
}

/// Compute the canonical DB path from `config`.
pub(crate) fn db_path_for(config: &Config) -> PathBuf {
    config.workspace_dir.join(DB_DIR).join(DB_FILE)
}

/// Delete stale WAL/SHM side-files (`<db>-shm`, `<db>-wal`) that can block a
/// clean DB open after a crash. Logs what was deleted and returns `true` if
/// anything was removed.
///
/// SQLite names these files `<db_path>-shm` and `<db_path>-wal`.
/// For `chunks.db` that is `chunks.db-shm` / `chunks.db-wal`.
pub(crate) fn try_cleanup_stale_files(db_path: &std::path::Path) -> bool {
    let mut cleaned = false;
    for suffix in &["-shm", "-wal"] {
        // Build the side-file path: append suffix to the full db filename.
        let side = {
            let mut p = db_path.to_path_buf();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            p.set_file_name(format!("{name}{suffix}"));
            p
        };
        if side.exists() {
            match std::fs::remove_file(&side) {
                Ok(()) => {
                    log::warn!("[memory_tree] removed stale side-file: {}", side.display());
                    cleaned = true;
                }
                Err(e) => {
                    log::warn!(
                        "[memory_tree] failed to remove stale side-file {}: {e}",
                        side.display()
                    );
                }
            }
        }
    }
    cleaned
}

/// Run the full one-time DB initialisation (journal mode, schema, migrations)
/// against an already-open `Connection`. Used by `get_or_init_connection`.
fn init_db(conn: &Connection, config: &Config) -> Result<()> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .context("Failed to configure memory_tree busy timeout")?;
    // SQLite resets `foreign_keys` to off on every new connection. The
    // ConnectionCache holds one cached `Connection` per DB path, so
    // setting it here (alongside the rest of init) is the per-connection
    // surface — fast-path callers reuse the cached conn with FKs already
    // on.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to enable memory_tree foreign_keys pragma")?;
    // memory_tree runs the TRUNCATE rollback journal (see `apply_schema`), so
    // crash-safety requires synchronous=FULL — NORMAL is only corruption-safe
    // under WAL. Set explicitly so a future global default can't weaken it.
    conn.execute_batch("PRAGMA synchronous = FULL;")
        .context("Failed to set memory_tree synchronous=FULL")?;
    apply_schema(conn)?;
    // #1574 §7: one-shot, version-gated legacy→sidecar embedding migration.
    migrate_legacy_embeddings_to_sidecar(conn, config)?;
    Ok(())
}

fn apply_schema(conn: &Connection) -> Result<()> {
    // Note: `init_db` runs the `#1574 §7` legacy→sidecar embedding migration
    // after this returns, so the dim-equal copy step is not duplicated here.
    // memory_tree uses the TRUNCATE rollback journal, NOT WAL. WAL's `-shm`
    // shared-memory index + `-wal` checkpoint machinery are the root of the
    // cold-start IOERR_SHMMAP (macOS) / IOERR_TRUNCATE (Windows, AV-held
    // handles) failures (Sentry TAURI-RUST-EV / TAURI-RUST-X1). All tree
    // access serialises on the single cached `PMutex<Connection>` (see
    // `get_or_init_connection`), so WAL's only real benefit — concurrent
    // readers — is unused here, which makes WAL pure liability. The sibling
    // tree DBs (cron / vault / redirect_links) already run the default
    // rollback journal without issue.
    //
    // Requesting TRUNCATE on a database a prior release left in WAL mode
    // checkpoints the `-wal` back into the main file and removes the
    // `-wal`/`-shm` side-files, so this also migrates existing WAL databases
    // in place on upgrade.
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode=TRUNCATE", [], |row| row.get(0))
        .context("Failed to set memory_tree journal_mode=TRUNCATE")?;
    if !journal_mode.eq_ignore_ascii_case("truncate") {
        log::warn!(
            "[memory_tree] journal_mode is '{journal_mode}' after requesting TRUNCATE \
             — a prior WAL connection or a locked -wal may be blocking the switch"
        );
    }
    conn.execute_batch(SCHEMA)
        .context("Failed to initialize memory_tree schema")?;
    // Phase 2 migrations — additive, idempotent.
    add_column_if_missing(conn, "mem_tree_chunks", "embedding", "BLOB")?;
    // Phase 2 LLM-NER follow-up: per-chunk LLM importance signal +
    // human-readable reason. Both nullable; absence is treated as
    // "no LLM signal available" by readers.
    add_column_if_missing(conn, "mem_tree_score", "llm_importance", "REAL")?;
    add_column_if_missing(conn, "mem_tree_score", "llm_importance_reason", "TEXT")?;
    // Phase 3a (#709): parent-summary backlink on leaves.
    add_column_if_missing(conn, "mem_tree_chunks", "parent_summary_id", "TEXT")?;
    // Phase 4 (#710): sealed-summary embeddings for semantic rerank.
    add_column_if_missing(conn, "mem_tree_summaries", "embedding", "BLOB")?;
    // Async-pipeline lifecycle flag. Default 'admitted' so chunks ingested
    // before the queue migration stay queryable.
    add_column_if_missing(
        conn,
        "mem_tree_chunks",
        "lifecycle_status",
        "TEXT NOT NULL DEFAULT 'admitted'",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_lifecycle \
         ON mem_tree_chunks(lifecycle_status);",
    )
    .context("Failed to create mem_tree_chunks lifecycle index")?;
    // Phase MD-content (#TBD): pointer + integrity hash.
    add_column_if_missing(conn, "mem_tree_chunks", "content_path", "TEXT")?;
    add_column_if_missing(conn, "mem_tree_chunks", "content_sha256", "TEXT")?;
    // Phase MD-content (summaries).
    add_column_if_missing(conn, "mem_tree_summaries", "content_path", "TEXT")?;
    add_column_if_missing(conn, "mem_tree_summaries", "content_sha256", "TEXT")?;
    // Raw-archive pointer column.
    add_column_if_missing(conn, "mem_tree_chunks", "raw_refs_json", "TEXT")?;
    // #1365: is_user flag on indexed entity rows.
    add_column_if_missing(
        conn,
        "mem_tree_entity_index",
        "is_user",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

/// Whether `err` looks like one of the I/O error codes that warrant a
/// stale-file cleanup + single retry before giving up.
pub(crate) fn is_io_open_error(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(f, _)) = err.downcast_ref::<rusqlite::Error>() {
        return matches!(
            f.extended_code,
            SQLITE_CANTOPEN
                | SQLITE_IOERR_TRUNCATE
                | SQLITE_IOERR_SHMOPEN
                | SQLITE_IOERR_SHMSIZE
                | SQLITE_IOERR_SHMMAP
                | SQLITE_IOERR_IN_PAGE
        ) || f.code == rusqlite::ErrorCode::CannotOpen;
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("disk i/o error")
        || msg.contains("unable to open database file")
        || msg.contains("xshmmap")
        || msg.contains("truncate file")
}

/// Obtain (or lazily create) a cached connection for the workspace described
/// by `config`. Returns `Err` immediately when the circuit breaker is open.
pub(crate) fn get_or_init_connection(config: &Config) -> Result<Arc<PMutex<Connection>>> {
    let db_path = db_path_for(config);

    // ── Fast path: reject immediately if the breaker is open ─────────────
    {
        let breakers = conn_cache().breakers.lock();
        if let Some(breaker) = breakers.get(&db_path) {
            if breaker.is_open() {
                anyhow::bail!(
                    "[memory_tree] circuit breaker open for {}: too many consecutive init failures",
                    db_path.display()
                );
            }
        }
    }
    // ── Fast path: return cached connection if already initialised ────────
    {
        let guard = conn_cache().connections.lock();
        if let Some(conn) = guard.get(&db_path) {
            return Ok(Arc::clone(conn));
        }
    }

    // ── Slow path: serialise init per-path so concurrent workers don't
    //    all race into `open_and_init` on a cold DB.
    let init_lock = {
        let mut guard = conn_cache().init_locks.lock();
        guard
            .entry(db_path.clone())
            .or_insert_with(|| Arc::new(PMutex::new(())))
            .clone()
    };
    let _init_guard = init_lock.lock();

    // Re-check the cache once we hold the init lock — another thread
    // may have completed init while we were queued.
    {
        let guard = conn_cache().connections.lock();
        if let Some(conn) = guard.get(&db_path) {
            return Ok(Arc::clone(conn));
        }
    }

    log::debug!(
        "[memory_tree] opening and initialising DB at {}",
        db_path.display()
    );

    // Attempt to open + init the connection (dir creation is inside
    // `open_and_init` so every failure — including EEXIST on the dir —
    // reaches the circuit-breaker recording logic below). On certain I/O
    // errors (#2206) we clean up stale WAL/SHM side-files and retry once.
    let conn = open_and_init(&db_path, config).or_else(|first_err| {
        if is_io_open_error(&first_err) {
            log::warn!(
                "[memory_tree] I/O error on first open attempt ({}), cleaning stale files and retrying",
                first_err
            );
            try_cleanup_stale_files(&db_path);
            open_and_init(&db_path, config)
        } else {
            Err(first_err)
        }
    });

    match conn {
        Ok(conn) => {
            let arc_conn = Arc::new(PMutex::new(conn));
            conn_cache()
                .connections
                .lock()
                .insert(db_path.clone(), Arc::clone(&arc_conn));
            // Reset any prior failure counter now that init succeeded.
            if let Some(breaker) = conn_cache().breakers.lock().get(&db_path) {
                breaker.record_success();
            }
            log::debug!("[memory_tree] DB connection cached and ready");
            Ok(arc_conn)
        }
        Err(err) => {
            // Persist the breaker so the failure count accumulates across
            // calls even though no connection entry exists yet.
            let breaker = {
                let mut guard = conn_cache().breakers.lock();
                guard
                    .entry(db_path.clone())
                    .or_insert_with(|| Arc::new(CircuitBreaker::new()))
                    .clone()
            };
            let just_tripped = breaker.record_failure();
            if just_tripped {
                log::error!(
                    "[memory_tree] circuit breaker tripped for {}: {} consecutive init failures",
                    db_path.display(),
                    CB_THRESHOLD
                );
                let _ = crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::HealthChanged {
                        component: "memory_tree_db".to_string(),
                        healthy: false,
                        message: Some(format!(
                            "Schema init failed {CB_THRESHOLD} consecutive times"
                        )),
                    },
                );
            }
            Err(err)
        }
    }
}

/// Ensure the DB directory exists, open the SQLite file, and run the full
/// schema init sequence. All errors (dir creation, file open, schema init)
/// are returned as `Err` so callers can funnel them through the circuit
/// breaker logic in a single place.
fn open_and_init(db_path: &std::path::Path, config: &Config) -> Result<Connection> {
    let dir = db_path.parent().expect("db_path always has a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create memory_tree dir: {}", dir.display()))?;
    let conn = Connection::open(db_path)
        .with_context(|| format!("Failed to open memory_tree DB: {}", db_path.display()))?;
    init_db(&conn, config)
        .with_context(|| format!("Failed to init memory_tree schema: {}", db_path.display()))?;
    record_schema_apply(db_path);
    Ok(conn)
}

/// Remove the cached connection for `config`'s workspace (forces a fresh open
/// on the next `with_connection` call). Also clears the breaker so the next
/// open attempt is not immediately rejected. Does nothing if no entry exists.
#[allow(dead_code)]
pub(crate) fn invalidate_connection(config: &Config) {
    let db_path = db_path_for(config);
    conn_cache().connections.lock().remove(&db_path);
    conn_cache().breakers.lock().remove(&db_path);
    log::debug!(
        "[memory_tree] connection invalidated for {}",
        db_path.display()
    );
}

/// Clear the entire connection cache. For test isolation only.
#[cfg(test)]
pub(crate) fn clear_connection_cache() {
    conn_cache().connections.lock().clear();
    conn_cache().breakers.lock().clear();
    conn_cache().init_locks.lock().clear();
}

/// Open the memory_tree SQLite DB and run a closure against it.
///
/// Visible to sibling modules (e.g. `score::store`) so Phase 2 can reuse
/// the same connection setup / schema initialisation without duplication.
///
/// # Connection caching (#2206)
///
/// The underlying connection is initialised once per workspace path and then
/// reused from a process-level cache. Schema migrations run exactly once on
/// the first call for a given `config.workspace_dir`. Subsequent calls pay
/// only the cost of a `parking_lot::Mutex` lock and the closure itself.
///
/// `#[doc(hidden)] pub` (not `pub(crate)`) because the
/// `memory-tree-init-smoke` bin in `src/bin/` is a separate crate target
/// and must reach this entry point. It is NOT a stable API surface —
/// downstream crates should treat it as internal.
#[doc(hidden)]
pub fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let conn_arc = get_or_init_connection(config)?;
    let guard = conn_arc.lock();
    f(&guard)
}
