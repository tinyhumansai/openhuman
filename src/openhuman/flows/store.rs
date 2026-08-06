//! SQLite persistence for the `flows::` domain.
//!
//! Mirrors `src/openhuman/cron/store.rs`'s idiom: a `with_connection` helper
//! opens (and migrates) a dedicated SQLite database under the workspace, and
//! every public function takes `&Config` first and returns `anyhow::Result<T>`.
//!
//! Two tables:
//! - `flow_definitions` — one row per saved [`Flow`], with the graph stored as
//!   JSON text (`graph_json`).
//! - `flow_state` — a generic namespaced key/value table backing
//!   `tinyflows::caps::StateStore` (see `src/openhuman/flows/tinyflows/caps.rs`).
//!
//! There is deliberately **no** `flow_checkpoints` table here: the crate's own
//! `tinyagents::SqliteCheckpointer` owns checkpoint persistence in a separate
//! `checkpoints.db` (see `src/openhuman/flows/tinyflows/mod.rs::open_flow_checkpointer`).

use crate::openhuman::config::Config;
use crate::openhuman::flows::types::{
    FlowRevision, FlowRun, FlowRunStep, FlowSuggestion, SuggestionStatus,
};
use crate::openhuman::flows::Flow;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// Tracks which flows database files have already had their schema DDL (the
/// `CREATE TABLE`/`CREATE INDEX` batch, `PRAGMA journal_mode = WAL`, and the
/// `add_column_if_missing` migration probe) run against them in this process
/// (R-m8). `with_connection` deliberately keeps opening a fresh, lightweight
/// `rusqlite::Connection` per call — `Connection` is `!Sync`, so caching a
/// single shared one would need a process-wide mutex that serializes every
/// caller, including the concurrent-writer scenario [`upsert_flow_run_step`]'s
/// `BEGIN IMMEDIATE` fix (R-m1) depends on being able to run from independent
/// connections. What actually repeats needlessly on every open is the DDL
/// batch itself — including once per node per live run via
/// `upsert_flow_run_step`. Gating just that batch behind a per-path
/// "already initialized" set keeps it to one execution per process per
/// database file while every call still gets its own connection.
///
/// Keyed by path rather than a single flag: tests each open an independent
/// per-`TempDir` workspace within the same test binary, and a bare
/// `OnceLock<()>` would silently skip schema creation for every database path
/// after the first test to run in the process.
static INITIALIZED_SCHEMAS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// Runs the one-time schema DDL + migrations against `conn` unless `db_path`
/// has already been initialized in this process (see [`INITIALIZED_SCHEMAS`]).
/// Only marks `db_path` as initialized *after* [`init_schema`] succeeds, so a
/// transient failure (e.g. disk I/O) is retried on the next call rather than
/// permanently wedging the store into believing a schema exists that was
/// never created.
///
/// **Trust, but verify.** A cache hit is confirmed against the file actually on
/// disk before it is honoured. Before this gating existed, the DDL ran on every
/// `with_connection` call, so a database deleted or replaced at runtime — a
/// workspace reset, a manual deletion, a disk-recovery restore — self-healed on
/// the very next call: `Connection::open` silently creates a fresh empty file,
/// and `CREATE TABLE IF NOT EXISTS` immediately repopulated it. Caching removes
/// that safety net: the set still says "initialized" while the file behind it is
/// empty, so every subsequent query fails with `no such table` until the process
/// restarts. One indexed `sqlite_master` lookup is far cheaper than the ~11
/// statement DDL batch and restores the self-healing, so it is paid on each hit
/// rather than trusting a cache entry that the filesystem may have invalidated.
fn ensure_schema_initialized(conn: &Connection, db_path: &Path) -> Result<()> {
    use rusqlite::OptionalExtension;

    let initialized = INITIALIZED_SCHEMAS.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let guard = initialized
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.contains(db_path) {
            let schema_present: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'flow_definitions'",
                    [],
                    |_| Ok(true),
                )
                .optional()
                .context("Failed to probe flows schema presence")?
                .unwrap_or(false);
            if schema_present {
                return Ok(());
            }
            tracing::warn!(
                target: "flows",
                db = %db_path.display(),
                "[flows] schema cached as initialized but the database has no tables (deleted or replaced at runtime?) — re-running schema init"
            );
        }
    }
    init_schema(conn)?;
    let mut guard = initialized
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.insert(db_path.to_path_buf());
    Ok(())
}

/// The actual schema DDL: 5 `CREATE TABLE IF NOT EXISTS` + 6 `CREATE INDEX IF
/// NOT EXISTS` + `PRAGMA journal_mode = WAL` (a persistent db-file setting,
/// not per-connection — safe, and now guaranteed, to run only once) plus the
/// `require_approval` post-hoc column migration. Split out of
/// `with_connection` so [`ensure_schema_initialized`] can gate it (R-m8).
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS flow_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            graph_json  TEXT NOT NULL,
            enabled     INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            last_run_at TEXT,
            last_status TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_flow_definitions_enabled ON flow_definitions(enabled);

         CREATE TABLE IF NOT EXISTS flow_state (
            namespace TEXT NOT NULL,
            key       TEXT NOT NULL,
            value     TEXT NOT NULL,
            PRIMARY KEY (namespace, key)
         );

         CREATE TABLE IF NOT EXISTS flow_runs (
            id                      TEXT PRIMARY KEY,
            flow_id                 TEXT NOT NULL,
            thread_id               TEXT NOT NULL,
            status                  TEXT NOT NULL,
            started_at              TEXT NOT NULL,
            finished_at             TEXT,
            steps_json              TEXT NOT NULL DEFAULT '[]',
            pending_approvals_json  TEXT NOT NULL DEFAULT '[]',
            error                   TEXT,
            graph_hash              TEXT,
            FOREIGN KEY (flow_id) REFERENCES flow_definitions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_flow_runs_flow_id ON flow_runs(flow_id);
         CREATE INDEX IF NOT EXISTS idx_flow_runs_started_at ON flow_runs(started_at);

         CREATE TABLE IF NOT EXISTS flow_suggestions (
            id                     TEXT PRIMARY KEY,
            title                  TEXT NOT NULL,
            one_liner              TEXT NOT NULL,
            rationale              TEXT NOT NULL,
            trigger_hint           TEXT,
            steps_json             TEXT NOT NULL DEFAULT '[]',
            connections_json       TEXT NOT NULL DEFAULT '[]',
            slugs_json             TEXT NOT NULL DEFAULT '[]',
            build_prompt           TEXT NOT NULL,
            confidence             REAL NOT NULL DEFAULT 0,
            status                 TEXT NOT NULL DEFAULT 'new',
            created_at             TEXT NOT NULL,
            source_run_id          TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_flow_suggestions_status ON flow_suggestions(status);
         CREATE INDEX IF NOT EXISTS idx_flow_suggestions_created_at ON flow_suggestions(created_at);

         CREATE TABLE IF NOT EXISTS flow_revisions (
            id               TEXT PRIMARY KEY,
            flow_id          TEXT NOT NULL,
            graph_json       TEXT NOT NULL,
            name             TEXT NOT NULL,
            require_approval INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT NOT NULL,
            FOREIGN KEY (flow_id) REFERENCES flow_definitions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_flow_revisions_flow_id ON flow_revisions(flow_id, created_at);",
    )
    .context("Failed to initialize flows schema")?;

    // `require_approval` (issue B2) — added post-hoc so a workspace created
    // before this column existed still opens cleanly. Mirrors
    // `cron::store`'s `add_column_if_missing` idiom.
    add_column_if_missing(
        conn,
        "flow_definitions",
        "require_approval",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    // T-M1 — added post-hoc so a workspace whose `flows.db` predates the
    // stale-approval graph pin still opens cleanly. A row written before this
    // migration reads back as `graph_hash IS NULL`, which `flows_resume`
    // treats as "unknown — allow, with a warning log" (see its doc), never as
    // a hard refusal, so upgrading mid-park cannot strand an in-flight
    // approval.
    add_column_if_missing(conn, "flow_runs", "graph_hash", "TEXT")?;

    Ok(())
}

/// Opens (creating/migrating as needed — once per process per database file,
/// see [`ensure_schema_initialized`]) the flows SQLite database and runs `f`
/// against the connection.
fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("flows").join("flows.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create flows directory: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open flows DB: {}", db_path.display()))?;

    // Per-connection pragmas: NOT persisted in the database file, so these
    // must be reapplied on every open regardless of the schema-init cache
    // below. `busy_timeout` retries (rather than immediately erroring
    // `SQLITE_BUSY`) when a concurrent writer holds the lock — including this
    // store's own `BEGIN IMMEDIATE` step upsert (R-m1); `foreign_keys` is
    // required on every connection for the `ON DELETE CASCADE` FKs to be
    // enforced.
    conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")
        .context("Failed to set flows DB connection pragmas")?;

    ensure_schema_initialized(&conn, &db_path)?;

    tracing::debug!(db = %db_path.display(), "[flows] store opened");

    f(&conn)
}

/// Adds `name` to `table` if it isn't already present, tolerating the race
/// where a concurrent process adds the same column between the `PRAGMA`
/// check and the `ALTER TABLE`. Mirrors `cron::store::add_column_if_missing`
/// (kept per-domain rather than shared — each store owns its own connection
/// helper and this is a handful of lines).
fn add_column_if_missing(conn: &Connection, table: &str, name: &str, sql_type: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(1)?;
        if col_name == name {
            return Ok(());
        }
    }
    drop(rows);
    drop(stmt);

    match conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {name} {sql_type}"),
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(err, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            tracing::debug!(
                "[flows] column {table}.{name} already exists (concurrent migration): {err}"
            );
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("Failed to add {table}.{name}")),
    }
}

/// Shared column list for every `flow_definitions` SELECT — keeps
/// [`map_flow_row`]'s positional `row.get(N)` calls in sync with the query.
const FLOW_DEFINITION_COLUMNS: &str = "id, name, graph_json, enabled, created_at, updated_at, \
     last_run_at, last_status, require_approval";

/// Inserts or fully replaces a flow definition row.
pub fn upsert_flow(config: &Config, flow: &Flow) -> Result<()> {
    let graph_json = serde_json::to_string(&flow.graph).context("Failed to serialize graph")?;
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO flow_definitions
                (id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status, require_approval)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                graph_json = excluded.graph_json,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at,
                last_run_at = excluded.last_run_at,
                last_status = excluded.last_status,
                require_approval = excluded.require_approval",
            params![
                flow.id,
                flow.name,
                graph_json,
                if flow.enabled { 1 } else { 0 },
                flow.created_at,
                flow.updated_at,
                flow.last_run_at,
                flow.last_status,
                if flow.require_approval { 1 } else { 0 },
            ],
        )
        .context("Failed to upsert flow definition")?;
        tracing::debug!(flow_id = %flow.id, "[flows] upserted flow definition");
        Ok(())
    })
}

/// Duplicates an existing [`Flow`] into a fresh row: same graph +
/// `require_approval`, a new id/timestamps, the given `new_name`, and
/// **`enabled = false`** so the copy never auto-fires (no schedule/app_event
/// trigger is bound while disabled — the caller relies on this to keep a
/// duplicate inert until explicitly enabled). `last_run_at`/`last_status` are
/// reset to `None` — run history does not carry over. Returns the persisted
/// copy.
pub fn insert_duplicate_flow(config: &Config, source: &Flow, new_name: String) -> Result<Flow> {
    let now = Utc::now().to_rfc3339();
    let flow = Flow {
        id: Uuid::new_v4().to_string(),
        name: new_name,
        enabled: false,
        graph: source.graph.clone(),
        created_at: now.clone(),
        updated_at: now,
        last_run_at: None,
        last_status: None,
        require_approval: source.require_approval,
    };
    upsert_flow(config, &flow)?;
    tracing::debug!(target: "flows", source_id = %source.id, new_id = %flow.id, "[flows] inserted duplicate flow (disabled)");
    Ok(flow)
}

/// Creates a brand-new [`Flow`] row from a name + validated graph, stamping
/// fresh id/timestamps, and returns the persisted record.
///
/// `enabled` is decided by the caller ([`crate::openhuman::flows::ops::flows_create`],
/// issue B29 — save/enable safety): a graph with an automatic trigger
/// (`schedule` / `app_event` / `webhook`) is created disabled so it cannot
/// silently arm itself live and unattended; a `manual`-triggered graph is
/// created enabled since it only ever runs on explicit `flows_run`.
pub fn create_flow(
    config: &Config,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
    enabled: bool,
) -> Result<Flow> {
    let now = Utc::now().to_rfc3339();
    let flow = Flow {
        id: Uuid::new_v4().to_string(),
        name,
        enabled,
        graph,
        created_at: now.clone(),
        updated_at: now,
        last_run_at: None,
        last_status: None,
        require_approval,
    };
    upsert_flow(config, &flow)?;
    Ok(flow)
}

/// Loads one flow by id, running its stored `graph_json` through
/// `tinyflows::migrate::migrate` before deserializing so a graph persisted
/// under an older `schema_version` is upgraded on read.
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_DEFINITION_COLUMNS} FROM flow_definitions WHERE id = ?1"
        ))?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(map_flow_row(row)?)),
            None => Ok(None),
        }
    })
}

/// Runs a `flow_definitions` SELECT and splits its rows into successfully
/// decoded [`Flow`]s and a count of rows that failed to parse/migrate
/// (R-M4).
///
/// **Skip-and-log, not fail-the-whole-query.** Before this, `list_flows` /
/// `list_enabled_flows` did `flows.push(row?)`, so a single corrupt or
/// newer-schema-than-this-build `graph_json` (e.g. a user downgrades after
/// running a newer build that persisted a graph `tinyflows::migrate::migrate`
/// cannot step backward) hard-failed the *entire* query — bricking every
/// `flows_list`, every `app_event` trigger dispatch (which is driven by
/// `list_enabled_flows`, see `bus.rs::handle_app_event`), and the boot
/// `reconcile_schedule_triggers_on_boot` sweep, all because of one bad row.
/// Mirrors the posture `draft_store::list_drafts` already uses. The returned
/// skip count is **not** swallowed here — it is the caller's job to log/
/// surface it loudly (a silently short flow list is its own failure mode) —
/// but this function itself does log each skip at `warn` with the row's `id`
/// and the parse/migrate error, never the `graph_json` payload.
fn list_flow_rows(conn: &Connection, where_clause: &str) -> Result<(Vec<Flow>, usize)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FLOW_DEFINITION_COLUMNS} FROM flow_definitions {where_clause} \
         ORDER BY created_at ASC"
    ))?;
    let mut rows = stmt.query([])?;
    let mut flows = Vec::new();
    let mut skipped = 0usize;
    while let Some(row) = rows.next()? {
        match map_flow_row(row) {
            Ok(flow) => flows.push(flow),
            Err(e) => {
                skipped += 1;
                let id: String = row.get(0).unwrap_or_else(|_| "<unknown>".to_string());
                tracing::warn!(
                    target: "flows",
                    flow_id = %id,
                    error = %e,
                    "[flows] skipping corrupt or unmigratable flow_definitions row \
                     (graph_json failed to parse/migrate)"
                );
            }
        }
    }
    Ok((flows, skipped))
}

/// Lists all saved flows, migrating each graph on read (see [`get_flow`]).
///
/// Returns `(flows, skipped)` — `skipped` is the number of rows that could
/// not be decoded and were left out of `flows` (R-M4). Callers must not treat
/// a non-zero `skipped` as a reason to fail; they must surface it loudly
/// instead (see [`list_flow_rows`]).
pub fn list_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    with_connection(config, |conn| list_flow_rows(conn, ""))
}

/// Lists only enabled flows, migrating each graph on read (see [`get_flow`]).
///
/// Used by `flows::bus::FlowTriggerSubscriber` to match an inbound
/// `ComposioTriggerReceived` event against every enabled `app_event` flow —
/// scanning the (small) enabled set once per event is simpler and cheap
/// enough at expected flow counts; a dedicated toolkit/trigger_slug index is
/// a later optimization if this ever shows up as a bottleneck.
///
/// Returns `(flows, skipped)` — see [`list_flows`]. A corrupt row here must
/// not take down `app_event` dispatch for every *other* enabled flow (R-M4).
pub fn list_enabled_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    with_connection(config, |conn| list_flow_rows(conn, "WHERE enabled = 1"))
}

/// Deletes a flow by id. Returns an error if no such flow exists.
pub fn remove_flow(config: &Config, id: &str) -> Result<()> {
    let changed = with_connection(config, |conn| {
        conn.execute("DELETE FROM flow_definitions WHERE id = ?1", params![id])
            .context("Failed to delete flow definition")
    })?;
    if changed == 0 {
        anyhow::bail!("flow '{id}' not found");
    }
    tracing::debug!(flow_id = %id, "[flows] removed flow definition");
    Ok(())
}

/// Toggles a flow's `enabled` flag, returning the updated record.
pub fn set_enabled(config: &Config, id: &str, enabled: bool) -> Result<Flow> {
    let now = Utc::now().to_rfc3339();
    let changed = with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_definitions SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![if enabled { 1 } else { 0 }, now, id],
        )
        .context("Failed to update flow enabled state")
    })?;
    if changed == 0 {
        anyhow::bail!("flow '{id}' not found");
    }
    tracing::debug!(flow_id = %id, enabled, "[flows] set_enabled");
    get_flow(config, id)?.ok_or_else(|| anyhow::anyhow!("flow '{id}' not found after update"))
}

/// How many revision snapshots to retain per flow (audit F6). Older ones are
/// pruned on each new capture.
const MAX_REVISIONS_PER_FLOW: usize = 20;

/// Failure modes of [`update_flow_graph`] that the caller must distinguish:
/// a genuine not-found, an optimistic-concurrency conflict (carrying the
/// current server flow so the UI can diff/reload), or a store error.
#[derive(Debug)]
pub enum FlowUpdateError {
    /// No flow with that id exists.
    NotFound,
    /// The flow changed since `expected_updated_at` was observed — the write
    /// was refused to avoid clobbering. Carries the current server flow.
    Conflict(Box<Flow>),
    /// An underlying store failure.
    Store(anyhow::Error),
}

impl std::fmt::Display for FlowUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "flow not found"),
            Self::Conflict(_) => write!(f, "flow changed since it was loaded"),
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

/// Replaces a flow's name/graph/`require_approval` (re-validated by the caller
/// before this is invoked) in place, bumping `updated_at`, capturing the prior
/// graph as a revision, and enforcing optimistic concurrency.
///
/// When `expected_updated_at` is `Some`, the write is refused with
/// [`FlowUpdateError::Conflict`] (carrying the current server flow) if the
/// flow's `updated_at` no longer matches — so an agent save and a concurrent
/// canvas save can't silently clobber each other. `None` keeps the prior
/// last-write-wins behaviour for callers that don't track a version.
///
/// `enabled_override`, when `Some`, forces the persisted `enabled` flag to
/// that value in the *same* guarded `UPDATE` as the graph/name/
/// `require_approval` write. `None` leaves `enabled` untouched (falls back to
/// the freshly re-read `current.enabled`), matching the previous behaviour
/// for every other caller.
///
/// `force_disarm_if_automatic`, when `true`, unconditionally disarms
/// (`enabled: false`) if the resulting graph (`graph`) has an automatic
/// trigger — used by `ops::flows_update_disarming_automatic` for remote
/// authoring surfaces.
///
/// **R-m2:** independent of `force_disarm_if_automatic`, this ALWAYS disarms
/// on a manual/none → automatic trigger transition (the B29 Rule 1 analogue)
/// — computed here, against the row this call just re-read
/// (`current.graph`), rather than trusting a transition flag the caller
/// derived from an earlier, possibly-stale read. `update_flow_graph`'s own
/// guarded `UPDATE` below keys its `WHERE` clause on this exact `current`
/// row, so this is the only read of "was it automatic before" that can't
/// have gone stale between computing the decision and writing it. An
/// `enabled_override` supplied by the caller can never re-arm a graph this
/// check disarms — the disarm always wins.
pub fn update_flow_graph(
    config: &Config,
    id: &str,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
    enabled_override: Option<bool>,
    force_disarm_if_automatic: bool,
    expected_updated_at: Option<&str>,
) -> std::result::Result<Flow, FlowUpdateError> {
    let current = get_flow(config, id)
        .map_err(FlowUpdateError::Store)?
        .ok_or(FlowUpdateError::NotFound)?;

    // Optimistic-concurrency check: refuse if the flow moved on since the
    // caller observed `expected_updated_at`.
    if let Some(expected) = expected_updated_at {
        if current.updated_at != expected {
            return Err(FlowUpdateError::Conflict(Box::new(current)));
        }
    }

    // R-m2: `was_auto` MUST come from `current` (just re-read above, right
    // before the guarded UPDATE below), never from a caller-observed
    // snapshot — a concurrent write between an ops-level read and this call
    // would otherwise let a manual→automatic transition slip past
    // undetected and persist `enabled: true` on an automatic-trigger graph.
    let now_auto = super::ops::trigger_is_automatic(&graph);
    let was_auto = super::ops::trigger_is_automatic(&current.graph);
    let is_manual_to_auto_transition = now_auto && !was_auto;
    let forced_automatic_disarm = force_disarm_if_automatic && now_auto;
    let auto_disarm = is_manual_to_auto_transition || forced_automatic_disarm;
    if auto_disarm {
        tracing::debug!(
            target: "flows",
            flow_id = %id,
            was_auto,
            now_auto,
            is_manual_to_auto_transition,
            forced_automatic_disarm,
            "[flows] update_flow_graph: disarming — automatic-trigger transition detected \
             against the freshly re-read row (R-m2)"
        );
    }

    let graph_json = serde_json::to_string(&graph)
        .context("Failed to serialize graph")
        .map_err(FlowUpdateError::Store)?;
    let prior_graph_json =
        serde_json::to_string(&current.graph).unwrap_or_else(|_| "null".to_string());
    let now = Utc::now().to_rfc3339();
    let new_enabled = if auto_disarm {
        false
    } else {
        enabled_override.unwrap_or(current.enabled)
    };

    with_connection(config, |conn| {
        // Guarded UPDATE keyed on the observed updated_at (race-safe even
        // without an explicit expected version) — a concurrent writer that
        // moved updated_at makes this match 0 rows. Targeted columns only, so a
        // concurrent set_enabled/record_run isn't clobbered (unless this call
        // itself carries an `enabled_override`, in which case `enabled` is
        // one of the targeted columns by design).
        let changed = conn
            .execute(
                "UPDATE flow_definitions SET name = ?1, graph_json = ?2, updated_at = ?3, \
                 require_approval = ?4, enabled = ?5 WHERE id = ?6 AND updated_at = ?7",
                params![
                    name,
                    graph_json,
                    now,
                    if require_approval { 1 } else { 0 },
                    if new_enabled { 1 } else { 0 },
                    id,
                    current.updated_at,
                ],
            )
            .context("Failed to update flow")?;
        if changed == 0 {
            // Someone raced us between the read and the write.
            anyhow::bail!("__conflict__");
        }
        // Capture the prior graph as a revision, then prune to the cap.
        let rev_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO flow_revisions (id, flow_id, graph_json, name, require_approval, \
             created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rev_id,
                id,
                prior_graph_json,
                current.name,
                if current.require_approval { 1 } else { 0 },
                now,
            ],
        )
        .context("Failed to record flow revision")?;
        conn.execute(
            "DELETE FROM flow_revisions WHERE flow_id = ?1 AND id NOT IN (\
                SELECT id FROM flow_revisions WHERE flow_id = ?1 \
                ORDER BY created_at DESC, id DESC LIMIT ?2)",
            params![id, MAX_REVISIONS_PER_FLOW as i64],
        )
        .context("Failed to prune flow revisions")?;
        Ok(())
    })
    .map_err(|e| {
        if e.to_string().contains("__conflict__") {
            // Re-read to hand back the current state.
            match get_flow(config, id) {
                Ok(Some(f)) => FlowUpdateError::Conflict(Box::new(f)),
                Ok(None) => FlowUpdateError::NotFound,
                Err(e) => FlowUpdateError::Store(e),
            }
        } else {
            FlowUpdateError::Store(e)
        }
    })?;

    get_flow(config, id)
        .map_err(FlowUpdateError::Store)?
        .ok_or(FlowUpdateError::NotFound)
}

/// Lists a flow's revision snapshots, newest first, up to `limit`.
pub fn list_revisions(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRevision>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, flow_id, graph_json, name, require_approval, created_at \
             FROM flow_revisions WHERE flow_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![flow_id, limit as i64], map_revision_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Fetches one revision by id (scoped to `flow_id`), or `None`.
pub fn revision_by_id(
    config: &Config,
    flow_id: &str,
    revision_id: &str,
) -> Result<Option<FlowRevision>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, flow_id, graph_json, name, require_approval, created_at \
             FROM flow_revisions WHERE flow_id = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query_map(params![flow_id, revision_id], map_revision_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    })
}

fn map_revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRevision> {
    let graph_str: String = row.get(2)?;
    let graph: serde_json::Value =
        serde_json::from_str(&graph_str).unwrap_or(serde_json::Value::Null);
    Ok(FlowRevision {
        id: row.get(0)?,
        flow_id: row.get(1)?,
        graph,
        name: row.get(3)?,
        require_approval: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
    })
}

/// Records the outcome of a `flows_run` invocation onto the flow's summary
/// fields (`last_run_at` / `last_status`).
pub fn record_run(config: &Config, id: &str, status: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let changed = with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_definitions SET last_run_at = ?1, last_status = ?2 WHERE id = ?3",
            params![now, status, id],
        )
        .context("Failed to record flow run")
    })?;
    if changed == 0 {
        anyhow::bail!("flow '{id}' not found");
    }
    tracing::debug!(flow_id = %id, status, "[flows] recorded run");
    Ok(())
}

fn map_flow_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Flow> {
    let graph_raw: String = row.get(2)?;
    let raw_value: serde_json::Value =
        serde_json::from_str(&graph_raw).map_err(sql_conversion_error)?;
    let migrated = tinyflows::migrate::migrate(raw_value).map_err(sql_conversion_error)?;
    let graph: tinyflows::model::WorkflowGraph =
        serde_json::from_value(migrated).map_err(sql_conversion_error)?;

    Ok(Flow {
        id: row.get(0)?,
        name: row.get(1)?,
        graph,
        enabled: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        last_run_at: row.get(6)?,
        last_status: row.get(7)?,
        require_approval: row.get::<_, i64>(8)? != 0,
    })
}

fn sql_conversion_error<E: std::error::Error + Send + Sync + 'static>(err: E) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

/// Loads a value from the `flow_state` KV table, scoped to `namespace`.
///
/// Backs `tinyflows::caps::StateStore::load` via
/// `src/openhuman/flows/tinyflows/caps.rs::FlowStateStore`.
pub fn kv_get(config: &Config, namespace: &str, key: &str) -> Result<Option<serde_json::Value>> {
    with_connection(config, |conn| {
        let mut stmt =
            conn.prepare("SELECT value FROM flow_state WHERE namespace = ?1 AND key = ?2")?;
        let mut rows = stmt.query(params![namespace, key])?;
        match rows.next()? {
            Some(row) => {
                let raw: String = row.get(0)?;
                let value: serde_json::Value =
                    serde_json::from_str(&raw).map_err(sql_conversion_error)?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    })
}

/// Stores a value into the `flow_state` KV table, scoped to `namespace`.
///
/// Backs `tinyflows::caps::StateStore::store` via
/// `src/openhuman/flows/tinyflows/caps.rs::FlowStateStore`.
pub fn kv_set(
    config: &Config,
    namespace: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let raw = serde_json::to_string(value).context("Failed to serialize flow state value")?;
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO flow_state (namespace, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(namespace, key) DO UPDATE SET value = excluded.value",
            params![namespace, key, raw],
        )
        .context("Failed to store flow state value")?;
        Ok(())
    })
}

/// Deletes one key from the `flow_state` KV table, scoped to `namespace`.
/// A no-op (not an error) when the key doesn't exist.
///
/// Used by `flows::bus::DedupCommitSubscriber` (issue #5263 PR2) to clear a
/// `dedup` node's `tentative` key set once a run's outcome has been settled —
/// preferred over `kv_set(.., json!([]))` because an absent key reads back as
/// `None` (an unambiguous "nothing pending"), matching what a fresh flow that
/// never ran a dedup node also reads back as.
pub fn kv_delete(config: &Config, namespace: &str, key: &str) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "DELETE FROM flow_state WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
        )
        .context("Failed to delete flow state value")?;
        Ok(())
    })
}

/// Shared column list for every `flow_runs` SELECT — keeps
/// [`map_flow_run_row`]'s positional `row.get(N)` calls in sync.
const FLOW_RUN_COLUMNS: &str = "id, flow_id, thread_id, status, started_at, finished_at, \
     steps_json, pending_approvals_json, error, graph_hash";

/// Default per-flow run-history retention cap: how many of the most-recent runs
/// a single flow keeps before older *terminal* runs are pruned on the next
/// insert (and by the manual `flows_prune_runs` sweep). Bounds unbounded
/// `flow_runs` growth for a hot, frequently-triggered flow while keeping enough
/// history for the run-history inspector.
///
/// Non-terminal runs (`running`, `pending_approval`) are **never** pruned — a
/// parked `pending_approval` run must survive so a later `flows_resume` can find
/// it — so the effective row count for a flow may briefly exceed this cap by the
/// number of live/parked runs. See [`prune_flow_runs`].
pub const MAX_FLOW_RUNS_PER_FLOW: usize = 100;

/// Inserts the initial `"running"` row for a new `flows_run` / `flows_resume`
/// invocation. `id` and `thread_id` are the same value in practice (the
/// tinyflows checkpointer thread id doubles as the run's stable identifier),
/// kept as two columns because they answer two different questions (row
/// identity vs. the checkpointer key `flows_resume` needs).
pub fn insert_flow_run(
    config: &Config,
    id: &str,
    flow_id: &str,
    thread_id: &str,
    started_at: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO flow_runs (id, flow_id, thread_id, status, started_at)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![id, flow_id, thread_id, started_at],
        )
        .context("Failed to insert flow run")?;
        // Retention: prune older terminal runs for this flow on every new-run
        // insert, so `flow_runs` stays bounded for a hot flow. Same connection
        // as the insert — atomic w.r.t. this write. A pruning failure is not
        // fatal to the insert (the run itself matters more than trimming
        // history), so it's logged and swallowed.
        if let Err(e) = prune_flow_runs_conn(conn, flow_id, MAX_FLOW_RUNS_PER_FLOW) {
            tracing::warn!(target: "flows", flow_id, error = %e, "[flows] insert_flow_run: retention prune failed (insert kept)");
        }
        Ok(())
    })
}

/// Prunes a flow's run history down to at most `keep` of its most-recent runs,
/// deleting any row outside the newest-`keep` window whose `status` is NOT
/// `running` or `pending_approval` — that is every terminal status this store
/// can hold (`completed`, `completed_with_warnings`, `failed`, `cancelled`,
/// `interrupted`, and any future status this host doesn't recognize yet), not
/// just the `completed`/`failed`/`cancelled` trio. The two excluded statuses
/// are the only ones that are never deleted — a parked `pending_approval` run
/// must never be pruned out from under a pending `flows_resume`, and a
/// `running` row belongs to a live task. Returns the number of rows deleted.
///
/// `keep` is clamped to at least 1. Exposed for the manual `flows_prune_runs`
/// sweep; the new-run insert path calls the connection-scoped helper directly.
pub fn prune_flow_runs(config: &Config, flow_id: &str, keep: usize) -> Result<usize> {
    with_connection(config, |conn| prune_flow_runs_conn(conn, flow_id, keep))
}

/// Connection-scoped core of [`prune_flow_runs`] — see its doc. Kept separate so
/// the new-run insert path can prune inside its own `with_connection` block
/// without reopening the database.
fn prune_flow_runs_conn(conn: &Connection, flow_id: &str, keep: usize) -> Result<usize> {
    let keep = i64::try_from(keep.max(1)).context("Run retention cap overflow")?;
    let deleted = conn
        .execute(
            "DELETE FROM flow_runs
              WHERE flow_id = ?1
                AND status NOT IN ('running', 'pending_approval')
                AND id NOT IN (
                    SELECT id FROM flow_runs
                     WHERE flow_id = ?1
                     ORDER BY started_at DESC, id DESC
                     LIMIT ?2
                )",
            params![flow_id, keep],
        )
        .context("Failed to prune flow runs")?;
    if deleted > 0 {
        tracing::debug!(target: "flows", flow_id, deleted, keep, "[flows] pruned old terminal flow runs past retention cap");
    }
    Ok(deleted)
}

/// Finalizes a flow run row: settles its terminal `status`, `finished_at`,
/// reconstructed `steps`, `pending_approvals`, and (on failure) `error`.
/// Called once a `flows_run` / `flows_resume` invocation settles — including
/// the timeout / capability-error paths, so a row never gets stuck at
/// `"running"` when the process is still up.
///
/// **Guarded write (R-M2).** The `UPDATE` only matches a row that is still
/// live — `status IN ('running','pending_approval')` — mirroring the same
/// re-check [`expire_parked_runs`] and [`mark_run_interrupted`] already do.
/// Without it this was an unconditional `WHERE id = ?`, so a caller that read a
/// non-terminal status and then lost a race could overwrite a row that had
/// meanwhile settled: `flows_cancel_run` reads `running`, the live run finishes
/// `completed` and deregisters, `run_registry::cancel` returns `false`, and the
/// "not in flight" branch then relabels a fully-completed run (whose real side
/// effects fired) as `cancelled`. Returns whether a row was actually updated so
/// callers can log the no-op instead of silently believing the write landed.
///
/// `graph_hash` (T-M1) is `Some(hash)` only when this write is the one that
/// *parks* the row (`status == "pending_approval"`) — it pins the content hash
/// of the graph the checkpoint was taken against, so a later `flows_resume`
/// can refuse if `save_workflow` rewrote the flow in the meantime. Every other
/// write passes `None`, which clears any stale pin once the row leaves
/// `pending_approval` (a settled row has no further use for it).
pub fn finish_flow_run(
    config: &Config,
    id: &str,
    status: &str,
    finished_at: &str,
    steps: &[FlowRunStep],
    pending_approvals: &[String],
    error: Option<&str>,
    graph_hash: Option<&str>,
) -> Result<bool> {
    let steps_json = serde_json::to_string(steps).context("Failed to serialize flow run steps")?;
    let pending_json = serde_json::to_string(pending_approvals)
        .context("Failed to serialize flow run pending approvals")?;
    with_connection(config, |conn| {
        let updated = conn
            .execute(
                "UPDATE flow_runs SET status = ?1, finished_at = ?2, steps_json = ?3, \
                 pending_approvals_json = ?4, error = ?5, graph_hash = ?6 \
                 WHERE id = ?7 AND status IN ('running', 'pending_approval')",
                params![
                    status,
                    finished_at,
                    steps_json,
                    pending_json,
                    error,
                    graph_hash,
                    id
                ],
            )
            .context("Failed to finish flow run")?;
        Ok(updated > 0)
    })
}

/// Incrementally upserts a single [`FlowRunStep`] onto a live `flow_runs`
/// row's `steps_json`, keyed by `node_id` — used by the run observer
/// (`flows::observability::FlowRunObserver`) to persist each node's step **as
/// it finishes** (issue G2, live run observation) rather than only rebuilding
/// the whole step list at settle.
///
/// **`BEGIN IMMEDIATE`-guarded read-modify-write (R-m1).** Each call opens its
/// own connection (see `with_connection`), so without an explicit transaction
/// two observer callbacks firing for parallel branch nodes of the *same* run
/// can interleave: both read `steps_json = [A]`, one writes `[A,B]`, the other
/// writes `[A,C]` — B is silently lost from the live view, and lost for good,
/// since the post-hoc `settle_steps` reconstruction only refills a missing
/// node with `status: None` rather than recovering the real outcome/duration.
/// `BEGIN IMMEDIATE` takes SQLite's write lock up front (rather than only at
/// the final `UPDATE`, which is what a plain autocommit read-then-write would
/// do), so a concurrent upsert either waits (covered by this store's
/// `busy_timeout = 5000` connection pragma — see `with_connection`) or is
/// serialized behind it; there is no window in which both readers can observe
/// the same pre-write `steps_json`. Kept deliberately minimal (one SELECT, one
/// UPDATE) to bound how long the write lock is held.
///
/// A re-run of the same `node_id` (a retry, or a resumed run re-touching a
/// node) replaces its prior entry rather than duplicating it, so the
/// persisted list stays one entry per node. No-op if the run's start row
/// hasn't been inserted yet (nothing to update) — mirrors the best-effort
/// contract of the run-row writers in `flows::ops`.
pub fn upsert_flow_run_step(config: &Config, run_id: &str, step: &FlowRunStep) -> Result<()> {
    use rusqlite::OptionalExtension;
    with_connection(config, |conn| {
        with_immediate_transaction(conn, |conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT steps_json FROM flow_runs WHERE id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .optional()
                .context("Failed to read flow run steps for incremental upsert")?;
            let Some(raw) = existing else {
                tracing::debug!(target: "flows", run_id, node = %step.node_id, "[flows] upsert_flow_run_step: no run row yet — skipping incremental step persist");
                return Ok(());
            };
            let mut steps: Vec<FlowRunStep> = serde_json::from_str(&raw)
                .context("Failed to deserialize existing flow run steps")?;
            match steps.iter_mut().find(|s| s.node_id == step.node_id) {
                Some(slot) => *slot = step.clone(),
                None => steps.push(step.clone()),
            }
            let steps_json =
                serde_json::to_string(&steps).context("Failed to serialize flow run steps")?;
            conn.execute(
                "UPDATE flow_runs SET steps_json = ?1 WHERE id = ?2",
                params![steps_json, run_id],
            )
            .context("Failed to persist incremental flow run step")?;
            tracing::debug!(target: "flows", run_id, node = %step.node_id, step_count = steps.len(), "[flows] persisted incremental flow run step");
            Ok(())
        })
    })
}

/// Runs `f` inside a `BEGIN IMMEDIATE` / `COMMIT` transaction on `conn`,
/// rolling back on error. `BEGIN IMMEDIATE` (rather than the default deferred
/// `BEGIN`) acquires SQLite's write lock immediately instead of only at the
/// first write statement, which is what closes the read-then-write race
/// [`upsert_flow_run_step`] needs closed (R-m1). Issued as raw SQL via
/// `execute_batch` rather than `rusqlite::Connection::transaction` (which
/// needs `&mut Connection`) so this can compose with `with_connection`'s
/// `&Connection` closure signature used by every other store function.
fn with_immediate_transaction<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("Failed to begin immediate transaction")?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT")
                .context("Failed to commit transaction")?;
            Ok(value)
        }
        Err(e) => {
            if let Err(rollback_err) = conn.execute_batch("ROLLBACK") {
                tracing::warn!(target: "flows", error = %rollback_err, "[flows] failed to roll back transaction after error");
            }
            Err(e)
        }
    }
}

/// Expires every parked `pending_approval` run whose "parked since" timestamp
/// (`COALESCE(finished_at, started_at)` — a run's `finished_at` is stamped when
/// it pauses at a gate) is strictly older than `cutoff` (an RFC3339 instant),
/// transitioning it to a terminal `"cancelled"` status stamped `now` with
/// `error_msg`. Returns the `(run_id, flow_id)` of the runs **actually flipped**
/// so the caller can update the flow summary, publish `FlowRunFinished`, and
/// drop the durable checkpoint (issue G4 — parked-run TTL) for real settles
/// only.
///
/// **Candidates are not sweeps.** The `SELECT` and each row's guarded `UPDATE`
/// are separate statements on an autocommit connection (`with_connection` opens
/// a fresh connection per call, not a transaction spanning this function), so a
/// concurrent `mark_run_resuming` on another connection can land in between: the
/// row was `pending_approval` at `SELECT` time and no longer is when its own
/// `UPDATE` runs. The per-row `WHERE status = 'pending_approval'` re-check keeps
/// that row's data safe — but returning the unfiltered candidate list would let
/// the caller act on a run it never actually expired: dropping the checkpoint out
/// from under a resume that just claimed it, and publishing a terminal
/// `FlowRunFinished` for a run still executing. That false event is the worse
/// half, because the frontend de-dupes terminal events by `${flow_id}:${run_id}`
/// — so the run's real completion would later be discarded as an alias replay,
/// leaving a successful run displayed as cancelled. Only rows whose `UPDATE`
/// reports `changed > 0` are returned.
///
/// RFC3339 timestamps produced by `chrono::Utc::…to_rfc3339()` all carry the
/// same `+00:00` offset, so a lexicographic `<` is a valid chronological
/// comparison here. Best-effort by contract at the call site: the update runs
/// under the same WAL + `busy_timeout` connection as every other write.
pub fn expire_parked_runs(
    config: &Config,
    cutoff: &str,
    now: &str,
    error_msg: &str,
) -> Result<Vec<(String, String)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, flow_id FROM flow_runs
             WHERE status = 'pending_approval'
               AND COALESCE(finished_at, started_at) < ?1",
        )?;
        let stale: Vec<(String, String)> = stmt
            .query_map(params![cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);

        let mut swept = Vec::with_capacity(stale.len());
        for (run_id, flow_id) in stale {
            // Re-check the status in the WHERE so a run resumed/cancelled
            // between the SELECT and here is not clobbered, and keep only the
            // rows this sweep genuinely flipped — see the fn doc.
            let changed = conn
                .execute(
                    "UPDATE flow_runs SET status = 'cancelled', finished_at = ?1, error = ?2 \
                     WHERE id = ?3 AND status = 'pending_approval'",
                    params![now, error_msg, &run_id],
                )
                .context("Failed to expire parked flow run")?;
            if changed > 0 {
                swept.push((run_id, flow_id));
            } else {
                tracing::debug!(
                    target: "flows",
                    run_id = %run_id,
                    "[flows] TTL sweep: run left 'pending_approval' concurrently — not expiring it"
                );
            }
        }
        if !swept.is_empty() {
            tracing::info!(target: "flows", swept = swept.len(), "[flows] expired parked pending_approval runs past TTL");
        }
        Ok(swept)
    })
}

/// Lists the `(id, flow_id)` of every run persisted at `status = 'running'`
/// whose `started_at` is strictly **before** `started_before` (RFC3339). Used by
/// the boot-time orphan sweep (bug B42): after a crash/restart no in-process
/// task is executing these rows, so
/// [`crate::openhuman::flows::ops::sweep_orphaned_running_runs_on_boot`]
/// reconciles each one that isn't backed by a live in-flight run to a terminal
/// `'interrupted'` via [`mark_run_interrupted`].
///
/// The `started_before` floor is what makes the sweep provably unable to touch
/// a run **this** process started: the sweep passes the instant this process
/// first entered the flow-run lifecycle, and every row this process inserts is
/// stamped at or after that instant. Without it, the sweep's only guard is the
/// in-flight registry, which a row briefly escapes between `start_flow_run_row`
/// and `run_registry::register`. `started_at` is a fixed-shape UTC RFC3339
/// string, so the lexicographic `<` matches chronological order (same
/// comparison the parked-run TTL sweep already relies on).
pub fn list_running_run_ids(
    config: &Config,
    started_before: &str,
) -> Result<Vec<(String, String)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, flow_id FROM flow_runs WHERE status = 'running' AND started_at < ?1",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map(params![started_before], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })
}

/// Test-only unconditional status write, bypassing the
/// [`finish_flow_run`] liveness guard.
///
/// Production code must never do a terminal → terminal transition — that is the
/// corruption [`finish_flow_run`]'s `status IN ('running','pending_approval')`
/// predicate exists to prevent. But a couple of tests legitimately need to
/// *stage* a row at an arbitrary terminal status (`completed_with_warnings`,
/// `interrupted`) to exercise the guards that read it, and they previously did
/// so by calling `finish_flow_run` twice — which the guard now correctly
/// refuses. Staging is a fixture concern, so it gets a fixture-only door rather
/// than a weaker production write.
#[cfg(test)]
pub fn force_run_status_for_test(
    config: &Config,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_runs SET status = ?1, error = ?2 WHERE id = ?3",
            params![status, error, id],
        )
        .context("Failed to force flow run status (test fixture)")?;
        Ok(())
    })
}

/// Test-only fixture door: overwrites an existing flow row's `graph_json`
/// with arbitrary text, bypassing the normal `Flow`/`WorkflowGraph`-typed
/// write path entirely. Used to stage the corrupt-or-newer-schema-row
/// scenario `list_flows` / `list_enabled_flows` / boot reconciliation must
/// survive (R-M4) — same "staging is a fixture concern, so it gets a
/// fixture-only door" rationale as [`force_run_status_for_test`]. Real
/// production writes can never produce a row `map_flow_row` can't decode
/// (every write path serializes a validated `WorkflowGraph`), so there is no
/// non-test way to reach this state other than a cross-version downgrade.
#[cfg(test)]
pub fn force_corrupt_graph_json_for_test(
    config: &Config,
    flow_id: &str,
    raw_graph_json: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "UPDATE flow_definitions SET graph_json = ?1 WHERE id = ?2",
                params![raw_graph_json, flow_id],
            )
            .context("Failed to force corrupt graph_json (test fixture)")?;
        anyhow::ensure!(changed > 0, "flow '{flow_id}' not found (test fixture)");
        Ok(())
    })
}

/// Flips a parked `'pending_approval'` row to `'running'` for the duration of a
/// [`crate::openhuman::flows::ops::flows_resume`], guarded by a
/// `status = 'pending_approval'` predicate so a run cancelled or expired
/// concurrently is never revived. Returns `true` when a row was actually
/// flipped.
///
/// Without this flip the row stays `pending_approval` for the whole (up to
/// `FLOW_RUN_TIMEOUT_SECS`) resume, so
/// [`expire_parked_runs`]' TTL sweep still matches it: a run approved just
/// before its TTL would be relabelled `cancelled` and have its durable
/// checkpoint dropped **while the resume was actively executing approved
/// outbound nodes** (R-M1). Marking it `running` moves it out of the sweep's
/// predicate and into the same lifecycle state a `flows_run` occupies, which is
/// also what the boot orphan sweep already knows how to reconcile.
pub fn mark_run_resuming(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "UPDATE flow_runs SET status = 'running', finished_at = NULL, error = NULL \
                 WHERE id = ?1 AND status = 'pending_approval'",
                params![id],
            )
            .context("Failed to mark parked flow run as resuming")?;
        if changed > 0 {
            tracing::debug!(target: "flows", run_id = id, "[flows] marked parked run 'running' for the duration of the resume");
        }
        Ok(changed > 0)
    })
}

/// Reconciles a single orphaned `'running'` run row to a terminal
/// `'interrupted'` status stamped `now` (RFC3339) with `reason`, guarded by a
/// `status = 'running'` predicate so a run that settled or was resumed
/// concurrently is never clobbered. Returns `true` when a row was actually
/// flipped (bug B42 — cancellation-safe finalizer + boot sweep). Best-effort by
/// contract at the call site.
pub fn mark_run_interrupted(config: &Config, id: &str, now: &str, reason: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "UPDATE flow_runs SET status = 'interrupted', finished_at = ?1, error = ?2 \
                 WHERE id = ?3 AND status = 'running'",
                params![now, reason, id],
            )
            .context("Failed to reconcile orphaned running flow run")?;
        if changed > 0 {
            tracing::info!(target: "flows", run_id = id, "[flows] reconciled orphaned 'running' flow run to 'interrupted'");
        }
        Ok(changed > 0)
    })
}

/// Loads one flow run by id (== thread_id).
pub fn get_flow_run(config: &Config, id: &str) -> Result<Option<FlowRun>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_RUN_COLUMNS} FROM flow_runs WHERE id = ?1"
        ))?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(map_flow_run_row(row)?)),
            None => Ok(None),
        }
    })
}

/// Lists the most recent runs for a flow, newest first.
pub fn list_flow_runs(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>> {
    with_connection(config, |conn| {
        let lim = i64::try_from(limit.max(1)).context("Run history limit overflow")?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_RUN_COLUMNS} FROM flow_runs WHERE flow_id = ?1 \
             ORDER BY started_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![flow_id, lim], map_flow_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    })
}

/// List the most recent runs across ALL flows, newest first (the "All runs"
/// page). Uses the `idx_flow_runs_started_at` index for the ordering. Each
/// [`FlowRun`] carries its own `flow_id`, so the UI can group/label by flow.
pub fn list_all_flow_runs(config: &Config, limit: usize) -> Result<Vec<FlowRun>> {
    with_connection(config, |conn| {
        let lim = i64::try_from(limit.max(1)).context("Run history limit overflow")?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_RUN_COLUMNS} FROM flow_runs \
             ORDER BY started_at DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![lim], map_flow_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    })
}

fn map_flow_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRun> {
    let steps_raw: String = row.get(6)?;
    let steps: Vec<FlowRunStep> = serde_json::from_str(&steps_raw).map_err(sql_conversion_error)?;
    let pending_raw: String = row.get(7)?;
    let pending_approvals: Vec<String> =
        serde_json::from_str(&pending_raw).map_err(sql_conversion_error)?;

    Ok(FlowRun {
        id: row.get(0)?,
        flow_id: row.get(1)?,
        thread_id: row.get(2)?,
        status: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        steps,
        pending_approvals,
        error: row.get(8)?,
        graph_hash: row.get(9)?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// flow_suggestions — discovery-agent workflow suggestions (Flow Scout)
// ─────────────────────────────────────────────────────────────────────────────

/// Shared column list for every `flow_suggestions` SELECT — keeps
/// [`map_suggestion_row`]'s positional `row.get(N)` calls in sync with the query.
const FLOW_SUGGESTION_COLUMNS: &str = "id, title, one_liner, rationale, trigger_hint, steps_json, \
     connections_json, slugs_json, build_prompt, confidence, status, created_at, source_run_id";

/// Inserts a batch of freshly discovered suggestions.
///
/// **Dedupe-preserving upsert.** Each suggestion's `id` is a stable content
/// hash (see `discovery_tools`), so a re-run that re-proposes an identical idea
/// hits `ON CONFLICT(id)` and refreshes the *pitch* fields — **without**
/// resetting a `status` the user already set. This is the invariant that keeps a
/// dismissed idea dismissed and a built idea built across repeated discovery
/// runs: the `status` and `created_at` columns are deliberately excluded from
/// the `DO UPDATE SET` list. Returns the number of rows written.
pub fn upsert_suggestions(config: &Config, suggestions: &[FlowSuggestion]) -> Result<usize> {
    if suggestions.is_empty() {
        return Ok(0);
    }
    with_connection(config, |conn| {
        let mut written = 0usize;
        for s in suggestions {
            let steps_json = serde_json::to_string(&s.steps_outline)
                .context("Failed to serialize suggestion steps")?;
            let connections_json = serde_json::to_string(&s.suggested_connections)
                .context("Failed to serialize suggestion connections")?;
            let slugs_json = serde_json::to_string(&s.suggested_slugs)
                .context("Failed to serialize suggestion slugs")?;
            conn.execute(
                "INSERT INTO flow_suggestions
                    (id, title, one_liner, rationale, trigger_hint, steps_json,
                     connections_json, slugs_json, build_prompt, confidence, status,
                     created_at, source_run_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(id) DO UPDATE SET
                    title = excluded.title,
                    one_liner = excluded.one_liner,
                    rationale = excluded.rationale,
                    trigger_hint = excluded.trigger_hint,
                    steps_json = excluded.steps_json,
                    connections_json = excluded.connections_json,
                    slugs_json = excluded.slugs_json,
                    build_prompt = excluded.build_prompt,
                    confidence = excluded.confidence,
                    source_run_id = excluded.source_run_id",
                params![
                    s.id,
                    s.title,
                    s.one_liner,
                    s.rationale,
                    s.trigger_hint,
                    steps_json,
                    connections_json,
                    slugs_json,
                    s.build_prompt,
                    s.confidence,
                    s.status.as_str(),
                    s.created_at,
                    s.source_run_id,
                ],
            )
            .context("Failed to upsert flow suggestion")?;
            written += 1;
        }
        tracing::debug!(count = written, "[flows] upserted flow suggestions");
        Ok(written)
    })
}

/// Lists persisted suggestions, newest first, highest-confidence first within a
/// timestamp. When `status` is `Some`, only rows in that lifecycle state are
/// returned (the UI passes `New` to render the active "Suggested for you"
/// cards); `None` returns every status.
pub fn list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
    limit: usize,
) -> Result<Vec<FlowSuggestion>> {
    with_connection(config, |conn| {
        let lim = i64::try_from(limit.max(1)).context("Suggestion limit overflow")?;
        let mut out = Vec::new();
        match status {
            Some(st) => {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {FLOW_SUGGESTION_COLUMNS} FROM flow_suggestions WHERE status = ?1 \
                     ORDER BY created_at DESC, confidence DESC, id ASC LIMIT ?2"
                ))?;
                let rows = stmt.query_map(params![st.as_str(), lim], map_suggestion_row)?;
                for row in rows {
                    out.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {FLOW_SUGGESTION_COLUMNS} FROM flow_suggestions \
                     ORDER BY created_at DESC, confidence DESC, id ASC LIMIT ?1"
                ))?;
                let rows = stmt.query_map(params![lim], map_suggestion_row)?;
                for row in rows {
                    out.push(row?);
                }
            }
        }
        Ok(out)
    })
}

/// Updates one suggestion's lifecycle status (dismiss / mark built). Returns
/// `true` when a row matched, `false` when the id was unknown (already pruned).
pub fn set_suggestion_status(config: &Config, id: &str, status: SuggestionStatus) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "UPDATE flow_suggestions SET status = ?1 WHERE id = ?2",
                params![status.as_str(), id],
            )
            .context("Failed to update flow suggestion status")?;
        tracing::debug!(suggestion_id = %id, status = %status.as_str(), changed, "[flows] set suggestion status");
        Ok(changed > 0)
    })
}

fn map_suggestion_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowSuggestion> {
    let steps_raw: String = row.get(5)?;
    let steps_outline: Vec<String> =
        serde_json::from_str(&steps_raw).map_err(sql_conversion_error)?;
    let connections_raw: String = row.get(6)?;
    let suggested_connections: Vec<String> =
        serde_json::from_str(&connections_raw).map_err(sql_conversion_error)?;
    let slugs_raw: String = row.get(7)?;
    let suggested_slugs: Vec<String> =
        serde_json::from_str(&slugs_raw).map_err(sql_conversion_error)?;
    let status_raw: String = row.get(10)?;

    Ok(FlowSuggestion {
        id: row.get(0)?,
        title: row.get(1)?,
        one_liner: row.get(2)?,
        rationale: row.get(3)?,
        trigger_hint: row.get(4)?,
        steps_outline,
        suggested_connections,
        suggested_slugs,
        build_prompt: row.get(8)?,
        confidence: row.get(9)?,
        status: SuggestionStatus::from_str_lossy(&status_raw),
        created_at: row.get(11)?,
        source_run_id: row.get(12)?,
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
