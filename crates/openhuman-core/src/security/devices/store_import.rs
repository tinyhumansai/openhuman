//! One-shot import of the legacy `devices.db` `paired_devices` table into the
//! document layout (`store_documents.rs`): one `paired_devices` document per
//! `channel_id`, with the columns as fields and `revoked` as a boolean.
//! `shared_secret_encrypted` was never written by the store and is not
//! carried over. The legacy file is only ever opened by the storage importer
//! (on a scratch copy of it), never by the running store.

use crate::storage::rusqlite;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

use super::storage_schema::LEGACY_DEVICES;
use crate::config::Config;
use crate::storage::import::{table_exists, ImportDoc};

/// The legacy database, `<workspace>/devices/devices.db`.
pub(crate) fn db_path(config: &Config) -> std::path::PathBuf {
    config.workspace_dir.join("devices").join("devices.db")
}

/// Opens the legacy database (creating the table when absent, which only
/// happens on the importer's scratch copy) and runs `f` on it.
pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = db_path(config);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create devices dir: {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("open devices DB: {}", db_path.display()))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS paired_devices (
             channel_id                  TEXT PRIMARY KEY,
             label                       TEXT NOT NULL,
             device_pubkey               TEXT NOT NULL,
             core_session_token_hash     TEXT NOT NULL,
             shared_secret_encrypted     BLOB,
             created_at                  TEXT NOT NULL,
             last_seen_at                TEXT,
             revoked                     INTEGER NOT NULL DEFAULT 0
         );",
    )
    .context("devices DDL failed")?;
    f(&conn)
}

/// Reads every legacy row.
pub(crate) fn read(config: &Config) -> Result<Vec<ImportDoc>> {
    with_connection(config, read_rows)
}

fn read_rows(conn: &Connection) -> Result<Vec<ImportDoc>> {
    if !table_exists(conn, "paired_devices")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT channel_id, label, device_pubkey, core_session_token_hash,
                    created_at, last_seen_at, revoked
             FROM paired_devices ORDER BY rowid",
        )
        .context("[devices::import] prepare paired_devices")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ImportDoc {
                collection: LEGACY_DEVICES,
                id: row.get(0)?,
                doc: json!({
                    "label": row.get::<_, String>(1)?,
                    "device_pubkey": row.get::<_, String>(2)?,
                    "core_session_token_hash": row.get::<_, String>(3)?,
                    "created_at": row.get::<_, String>(4)?,
                    "last_seen_at": row
                        .get::<_, Option<String>>(5)?
                        .map_or(Value::Null, Value::String),
                    "revoked": row.get::<_, i64>(6)? != 0,
                }),
            })
        })
        .context("[devices::import] query paired_devices")?;
    rows.map(|row| row.context("[devices::import] decode a paired_devices row"))
        .collect()
}

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "store_import_tests.rs"]
mod tests;
