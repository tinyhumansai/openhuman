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
//!   `tinyflows::caps::StateStore` (see `src/openhuman/tinyflows/caps.rs`).
//!
//! There is deliberately **no** `flow_checkpoints` table here: the crate's own
//! `tinyagents::SqliteCheckpointer` owns checkpoint persistence in a separate
//! `checkpoints.db` (see `src/openhuman/tinyflows/mod.rs::open_flow_checkpointer`).

use crate::openhuman::config::Config;
use crate::openhuman::flows::types::{FlowRun, FlowRunStep, FlowSuggestion, SuggestionStatus};
use crate::openhuman::flows::Flow;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

/// Opens (creating/migrating as needed) the flows SQLite database and runs `f`
/// against the connection.
fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("flows").join("flows.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create flows directory: {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open flows DB: {}", db_path.display()))?;

    conn.execute_batch(
        // `busy_timeout` retries (rather than immediately erroring
        // `SQLITE_BUSY`) when a concurrent run/state write holds the lock; WAL
        // lets readers and a writer proceed together. Both are safe to re-issue
        // on every open (WAL is a persistent db-file setting; busy_timeout is
        // per-connection).
        "PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
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
         CREATE INDEX IF NOT EXISTS idx_flow_suggestions_created_at ON flow_suggestions(created_at);",
    )
    .context("Failed to initialize flows schema")?;

    // `require_approval` (issue B2) — added post-hoc so a workspace created
    // before this column existed still opens cleanly. Mirrors
    // `cron::store`'s `add_column_if_missing` idiom.
    add_column_if_missing(
        &conn,
        "flow_definitions",
        "require_approval",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

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

/// Lists all saved flows, migrating each graph on read (see [`get_flow`]).
pub fn list_flows(config: &Config) -> Result<Vec<Flow>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_DEFINITION_COLUMNS} FROM flow_definitions ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map([], map_flow_row)?;
        let mut flows = Vec::new();
        for row in rows {
            flows.push(row?);
        }
        Ok(flows)
    })
}

/// Lists only enabled flows, migrating each graph on read (see [`get_flow`]).
///
/// Used by `flows::bus::FlowTriggerSubscriber` to match an inbound
/// `ComposioTriggerReceived` event against every enabled `app_event` flow —
/// scanning the (small) enabled set once per event is simpler and cheap
/// enough at expected flow counts; a dedicated toolkit/trigger_slug index is
/// a later optimization if this ever shows up as a bottleneck.
pub fn list_enabled_flows(config: &Config) -> Result<Vec<Flow>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FLOW_DEFINITION_COLUMNS} FROM flow_definitions WHERE enabled = 1 \
             ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map([], map_flow_row)?;
        let mut flows = Vec::new();
        for row in rows {
            flows.push(row?);
        }
        Ok(flows)
    })
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

/// Replaces a flow's name/graph/`require_approval` (re-validated by the
/// caller before this is invoked) in place, bumping `updated_at`.
pub fn update_flow_graph(
    config: &Config,
    id: &str,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
) -> Result<Flow> {
    let graph_json = serde_json::to_string(&graph).context("Failed to serialize graph")?;
    let now = Utc::now().to_rfc3339();
    // Targeted UPDATE of only the editable columns, so a concurrent
    // `set_enabled` / `record_run` isn't clobbered by writing back a stale
    // `enabled` / `last_run_at` / `last_status` from a read-modify-write.
    let changed = with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_definitions SET name = ?1, graph_json = ?2, updated_at = ?3, \
             require_approval = ?4 WHERE id = ?5",
            params![
                name,
                graph_json,
                now,
                if require_approval { 1 } else { 0 },
                id
            ],
        )
        .context("Failed to update flow")
    })?;
    if changed == 0 {
        anyhow::bail!("flow '{id}' not found");
    }
    get_flow(config, id)?.ok_or_else(|| anyhow::anyhow!("flow '{id}' not found"))
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
/// `src/openhuman/tinyflows/caps.rs::FlowStateStore`.
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
/// `src/openhuman/tinyflows/caps.rs::FlowStateStore`.
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

/// Shared column list for every `flow_runs` SELECT — keeps
/// [`map_flow_run_row`]'s positional `row.get(N)` calls in sync.
const FLOW_RUN_COLUMNS: &str = "id, flow_id, thread_id, status, started_at, finished_at, \
     steps_json, pending_approvals_json, error";

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
/// deleting only **terminal** rows (`completed` / `failed` / `cancelled`) that
/// fall outside the newest-`keep` window. Non-terminal runs (`running`,
/// `pending_approval`) are never deleted — a parked `pending_approval` run must
/// never be pruned out from under a pending `flows_resume`, and a `running` row
/// belongs to a live task. Returns the number of rows deleted.
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
pub fn finish_flow_run(
    config: &Config,
    id: &str,
    status: &str,
    finished_at: &str,
    steps: &[FlowRunStep],
    pending_approvals: &[String],
    error: Option<&str>,
) -> Result<()> {
    let steps_json = serde_json::to_string(steps).context("Failed to serialize flow run steps")?;
    let pending_json = serde_json::to_string(pending_approvals)
        .context("Failed to serialize flow run pending approvals")?;
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_runs SET status = ?1, finished_at = ?2, steps_json = ?3, \
             pending_approvals_json = ?4, error = ?5 WHERE id = ?6",
            params![status, finished_at, steps_json, pending_json, error, id],
        )
        .context("Failed to finish flow run")?;
        Ok(())
    })
}

/// Incrementally upserts a single [`FlowRunStep`] onto a live `flow_runs`
/// row's `steps_json`, keyed by `node_id` — used by the run observer
/// (`flows::observability::FlowRunObserver`) to persist each node's step **as
/// it finishes** (issue G2, live run observation) rather than only rebuilding
/// the whole step list at settle.
///
/// Read-modify-write under a single connection (the WAL + `busy_timeout=5000`
/// this store opens with tolerates the concurrent settle write). A re-run of
/// the same `node_id` (a retry, or a resumed run re-touching a node) replaces
/// its prior entry rather than duplicating it, so the persisted list stays one
/// entry per node. No-op if the run's start row hasn't been inserted yet
/// (nothing to update) — mirrors the best-effort contract of the run-row
/// writers in `flows::ops`.
pub fn upsert_flow_run_step(config: &Config, run_id: &str, step: &FlowRunStep) -> Result<()> {
    use rusqlite::OptionalExtension;
    with_connection(config, |conn| {
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
        let mut steps: Vec<FlowRunStep> =
            serde_json::from_str(&raw).context("Failed to deserialize existing flow run steps")?;
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
}

/// Expires every parked `pending_approval` run whose "parked since" timestamp
/// (`COALESCE(finished_at, started_at)` — a run's `finished_at` is stamped when
/// it pauses at a gate) is strictly older than `cutoff` (an RFC3339 instant),
/// transitioning it to a terminal `"cancelled"` status stamped `now` with
/// `error_msg`. Returns the `(run_id, flow_id)` of each swept run so the caller
/// can update the flow summary + drop the durable checkpoint (issue G4 —
/// parked-run TTL).
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

        for (run_id, _flow_id) in &stale {
            // Re-check the status in the WHERE so a run resumed/cancelled
            // between the SELECT and here is not clobbered.
            conn.execute(
                "UPDATE flow_runs SET status = 'cancelled', finished_at = ?1, error = ?2 \
                 WHERE id = ?3 AND status = 'pending_approval'",
                params![now, error_msg, run_id],
            )
            .context("Failed to expire parked flow run")?;
        }
        if !stale.is_empty() {
            tracing::info!(target: "flows", swept = stale.len(), "[flows] expired parked pending_approval runs past TTL");
        }
        Ok(stale)
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
