//! SQLite-backed [`Checkpointer`] over openhuman's run ledger (issue #4249).
//!
//! tinyagents 1.1 ships a `SqliteCheckpointer`, but its `sqlite` feature pulls
//! `rusqlite 0.40` / `libsqlite3-sys 0.38`, which conflict with openhuman's own
//! `rusqlite 0.37` over the `links = "sqlite3"` native lib — so it cannot be
//! enabled. The crate's always-available `FileCheckpointer` works, but durable
//! orchestration state needs to stay queryable from the existing run-ledger
//! controllers (workflow/team/command-center read the same `sessions.db`).
//!
//! [`SqlRunLedgerCheckpointer`] is the stand-in: it implements the crate's
//! [`Checkpointer`] trait but persists each superstep-boundary snapshot as a row
//! in the `graph_checkpoints` table (one serialized
//! [`Checkpoint<State>`](tinyagents::graph::Checkpoint) per row, keyed by
//! `thread_id`, `seq`-ordered). It mirrors the reference `FileCheckpointer`
//! semantics exactly — append on `put`, latest-wins `get`, insertion-order
//! `list` — so any durable graph (delegation, workflow, teams) can checkpoint to
//! openhuman SQLite instead of the blocked crate backend.
//!
//! rusqlite is blocking, so every trait method runs the DB work inside
//! [`tokio::task::spawn_blocking`].

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tinyagents::graph::checkpoint::{Checkpoint, CheckpointMetadata, Checkpointer};
use tinyagents::harness::ids::CheckpointId;
use tinyagents::{Result as TaResult, TinyAgentsError};

use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::store::init_run_ledger_schema;
use crate::openhuman::session_db::with_connection;

/// A [`Checkpointer`] that persists graph checkpoints into the openhuman session
/// DB (`graph_checkpoints` table). Cheap to clone; clones address the same DB.
pub struct SqlRunLedgerCheckpointer<State> {
    config: Arc<Config>,
    _marker: PhantomData<fn() -> State>,
}

impl<State> SqlRunLedgerCheckpointer<State> {
    /// Build a checkpointer backed by the session DB resolved from `config`
    /// (`{workspace}/session_db/sessions.db`).
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }
}

impl<State> Clone for SqlRunLedgerCheckpointer<State> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            _marker: PhantomData,
        }
    }
}

/// Map an openhuman `anyhow` DB error onto the crate's checkpoint error variant.
fn db_err(context: &str, err: impl std::fmt::Display) -> TinyAgentsError {
    TinyAgentsError::Checkpoint(format!("sql run-ledger checkpointer: {context}: {err}"))
}

#[async_trait]
impl<State> Checkpointer<State> for SqlRunLedgerCheckpointer<State>
where
    State: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    async fn put(&self, checkpoint: Checkpoint<State>) -> TaResult<CheckpointId> {
        let config = self.config.clone();
        let id = CheckpointId::new(checkpoint.checkpoint_id.clone());
        let thread_id = checkpoint.thread_id.clone();
        let checkpoint_id = checkpoint.checkpoint_id.clone();
        let run_id = checkpoint.run_id.clone();
        let record_json =
            serde_json::to_string(&checkpoint).map_err(|e| db_err("encode record", e))?;

        tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                conn.execute(
                    "INSERT INTO graph_checkpoints \
                     (thread_id, checkpoint_id, run_id, record_json, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        thread_id,
                        checkpoint_id,
                        run_id,
                        record_json,
                        chrono::Utc::now().to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| db_err("put", e))
        })
        .await
        .map_err(|e| db_err("put join", e))??;

        Ok(id)
    }

    async fn get(
        &self,
        thread_id: &str,
        checkpoint_id: Option<&str>,
    ) -> TaResult<Option<Checkpoint<State>>> {
        let config = self.config.clone();
        let thread_id = thread_id.to_string();
        let checkpoint_id = checkpoint_id.map(str::to_string);

        let record_json: Option<String> = tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                // Latest-wins, matching the reference FileCheckpointer: scan in
                // reverse insertion order and take the first match.
                let row = match checkpoint_id {
                    Some(ref id) => conn
                        .query_row(
                            "SELECT record_json FROM graph_checkpoints \
                             WHERE thread_id = ?1 AND checkpoint_id = ?2 \
                             ORDER BY seq DESC LIMIT 1",
                            rusqlite::params![thread_id, id],
                            |r| r.get::<_, String>(0),
                        )
                        .ok(),
                    None => conn
                        .query_row(
                            "SELECT record_json FROM graph_checkpoints \
                             WHERE thread_id = ?1 ORDER BY seq DESC LIMIT 1",
                            rusqlite::params![thread_id],
                            |r| r.get::<_, String>(0),
                        )
                        .ok(),
                };
                Ok(row)
            })
            .map_err(|e| db_err("get", e))
        })
        .await
        .map_err(|e| db_err("get join", e))??;

        match record_json {
            Some(json) => {
                let checkpoint: Checkpoint<State> =
                    serde_json::from_str(&json).map_err(|e| db_err("decode record", e))?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    async fn list(&self, thread_id: &str) -> TaResult<Vec<CheckpointMetadata>> {
        let config = self.config.clone();
        let thread_id = thread_id.to_string();

        let records: Vec<String> = tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                let mut stmt = conn.prepare(
                    "SELECT record_json FROM graph_checkpoints \
                     WHERE thread_id = ?1 ORDER BY seq ASC",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![thread_id], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .map_err(|e| db_err("list", e))
        })
        .await
        .map_err(|e| db_err("list join", e))??;

        records
            .into_iter()
            .map(|json| {
                serde_json::from_str::<Checkpoint<State>>(&json)
                    .map(|c| c.to_metadata())
                    .map_err(|e| db_err("decode record", e))
            })
            .collect()
    }

    async fn list_threads(&self) -> TaResult<Vec<String>> {
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT thread_id FROM graph_checkpoints ORDER BY thread_id",
                )?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .map_err(|e| db_err("list_threads", e))
        })
        .await
        .map_err(|e| db_err("list_threads join", e))?
    }

    async fn delete_thread(&self, thread_id: &str) -> TaResult<()> {
        let config = self.config.clone();
        let thread_id = thread_id.to_string();
        tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                conn.execute(
                    "DELETE FROM graph_checkpoints WHERE thread_id = ?1",
                    rusqlite::params![thread_id],
                )?;
                Ok(())
            })
            .map_err(|e| db_err("delete_thread", e))
        })
        .await
        .map_err(|e| db_err("delete_thread join", e))?
    }

    async fn delete_checkpoints(&self, thread_id: &str, ids: &[String]) -> TaResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let config = self.config.clone();
        let thread_id = thread_id.to_string();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            with_connection(&config, |conn| {
                init_run_ledger_schema(conn)?;
                let mut removed = 0usize;
                // Bind each id individually — keeps the statement simple and avoids
                // SQLite's variadic-`IN` parameter packing.
                let mut stmt = conn.prepare(
                    "DELETE FROM graph_checkpoints WHERE thread_id = ?1 AND checkpoint_id = ?2",
                )?;
                for id in &ids {
                    removed += stmt.execute(rusqlite::params![thread_id, id])?;
                }
                Ok(removed)
            })
            .map_err(|e| db_err("delete_checkpoints", e))
        })
        .await
        .map_err(|e| db_err("delete_checkpoints join", e))?
    }
}
