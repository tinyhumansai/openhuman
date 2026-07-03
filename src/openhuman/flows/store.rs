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
         );",
    )
    .context("Failed to initialize flows schema")?;

    tracing::debug!(db = %db_path.display(), "[flows] store opened");

    f(&conn)
}

/// Inserts or fully replaces a flow definition row.
pub fn upsert_flow(config: &Config, flow: &Flow) -> Result<()> {
    let graph_json = serde_json::to_string(&flow.graph).context("Failed to serialize graph")?;
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO flow_definitions
                (id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                graph_json = excluded.graph_json,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at,
                last_run_at = excluded.last_run_at,
                last_status = excluded.last_status",
            params![
                flow.id,
                flow.name,
                graph_json,
                if flow.enabled { 1 } else { 0 },
                flow.created_at,
                flow.updated_at,
                flow.last_run_at,
                flow.last_status,
            ],
        )
        .context("Failed to upsert flow definition")?;
        tracing::debug!(flow_id = %flow.id, "[flows] upserted flow definition");
        Ok(())
    })
}

/// Creates a brand-new [`Flow`] row from a name + validated graph, stamping
/// fresh id/timestamps, and returns the persisted record.
pub fn create_flow(
    config: &Config,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
) -> Result<Flow> {
    let now = Utc::now().to_rfc3339();
    let flow = Flow {
        id: Uuid::new_v4().to_string(),
        name,
        enabled: true,
        graph,
        created_at: now.clone(),
        updated_at: now,
        last_run_at: None,
        last_status: None,
    };
    upsert_flow(config, &flow)?;
    Ok(flow)
}

/// Loads one flow by id, running its stored `graph_json` through
/// `tinyflows::migrate::migrate` before deserializing so a graph persisted
/// under an older `schema_version` is upgraded on read.
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status
             FROM flow_definitions WHERE id = ?1",
        )?;
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
        let mut stmt = conn.prepare(
            "SELECT id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status
             FROM flow_definitions ORDER BY created_at ASC",
        )?;
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

/// Replaces a flow's name/graph (re-validated by the caller before this is
/// invoked) in place, bumping `updated_at`.
pub fn update_flow_graph(
    config: &Config,
    id: &str,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
) -> Result<Flow> {
    let graph_json = serde_json::to_string(&graph).context("Failed to serialize graph")?;
    let now = Utc::now().to_rfc3339();
    // Targeted UPDATE of only the editable columns, so a concurrent
    // `set_enabled` / `record_run` isn't clobbered by writing back a stale
    // `enabled` / `last_run_at` / `last_status` from a read-modify-write.
    let changed = with_connection(config, |conn| {
        conn.execute(
            "UPDATE flow_definitions SET name = ?1, graph_json = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, graph_json, now, id],
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

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
