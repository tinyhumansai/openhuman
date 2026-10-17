//! One-shot import of the legacy `notifications.db` tables into the document
//! layout (`store_documents.rs`).
//!
//! | Legacy table | Collection | Id |
//! | --- | --- | --- |
//! | `integration_notifications` | `integration_notifications` | row id |
//! | `notification_settings` | `notification_settings` | provider |
//! | `core_notifications` | `core_notifications` | `core_id("local", id)` |
//!
//! Documents come from the store's own `to_doc` and field names. The legacy
//! `core_notifications` rows carry no workspace, so they take the fixed
//! `LOCAL_WORKSPACE` key the per-workspace default file filters on (not the
//! directory path, which changes when the workspace is moved).
//! (The content-dedup window needs no import: it lasts a minute.) The legacy
//! file is only ever opened here, by the storage importer, on a scratch copy.

use crate::storage::rusqlite;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::json;

#[path = "store_import_rows.rs"]
mod store_rows;
use store_rows::rows_to_notifications;

use super::storage_schema::{LEGACY_CORE, LEGACY_NOTIFICATIONS, LEGACY_SETTINGS};
use super::store_documents::LOCAL_WORKSPACE;
use super::types::IntegrationNotification;
use crate::config::Config;
use crate::storage::import::{table_exists, ImportDoc};

/// SQL schema of the legacy database, applied on every `with_connection` call.
const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS integration_notifications (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,
    account_id       TEXT,
    title            TEXT NOT NULL,
    body             TEXT NOT NULL,
    raw_payload      TEXT NOT NULL,
    importance_score REAL,
    triage_action    TEXT,
    triage_reason    TEXT,
    status           TEXT NOT NULL DEFAULT 'unread',
    received_at      TEXT NOT NULL,
    scored_at        TEXT
);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_provider
    ON integration_notifications(provider);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_status
    ON integration_notifications(status);
CREATE INDEX IF NOT EXISTS idx_integration_notifications_dedup
    ON integration_notifications(provider, account_id, title, body, received_at);

CREATE TABLE IF NOT EXISTS notification_settings (
    provider              TEXT PRIMARY KEY,
    enabled               INTEGER NOT NULL DEFAULT 1,
    importance_threshold  REAL NOT NULL DEFAULT 0.0,
    route_to_orchestrator INTEGER NOT NULL DEFAULT 1
);

-- #3805: core notification events (cron completions, webhook failures,
-- sub-agent results, triaged alerts, …) are broadcast over an in-memory
-- channel that drops the event when no client is connected. Persist them
-- here so a notification fired while the app is closed/minimised/disconnected
-- survives and is surfaced on the next app open. The full wire payload is
-- stored as JSON; `id` is the PK so re-publishes dedupe naturally.
CREATE TABLE IF NOT EXISTS core_notifications (
    id            TEXT PRIMARY KEY,
    payload       TEXT NOT NULL,
    timestamp_ms  INTEGER NOT NULL,
    read          INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_core_notifications_read
    ON core_notifications(read);
CREATE INDEX IF NOT EXISTS idx_core_notifications_ts
    ON core_notifications(timestamp_ms);
";

/// Open (and migrate) the notifications DB, then call `f` with the live
/// connection. Mirrors the `with_connection` helper in `cron/store.rs`.
pub(crate) fn db_path(config: &Config) -> std::path::PathBuf {
    config
        .workspace_dir
        .join("notifications")
        .join("notifications.db")
}

pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = db_path(config);

    tracing::trace!(
        path = %db_path.display(),
        "[notifications::store] opening DB connection"
    );

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "[notifications::store] failed to create dir {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "[notifications::store] failed to open DB at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch(SCHEMA)
        .context("[notifications::store] schema migration failed")?;

    tracing::trace!("[notifications::store] schema migration applied, running operation");
    f(&conn)
}

/// The v1 core-notification document id: the workspace is length-prefixed.
pub(crate) fn legacy_core_id(workspace: &str, event_id: &str) -> String {
    format!("{}:{workspace}/{event_id}", workspace.len())
}

/// A notification in its v1 document shape (`received_at`, `scored_at`
/// formatted times), which the owner's step converts.
fn legacy_doc(n: &IntegrationNotification) -> serde_json::Value {
    let mut doc = serde_json::Map::new();
    doc.insert("provider".into(), json!(n.provider));
    doc.insert("title".into(), json!(n.title));
    doc.insert("body".into(), json!(n.body));
    doc.insert("raw_payload".into(), json!(n.raw_payload.to_string()));
    doc.insert("status".into(), json!(n.status.as_str()));
    doc.insert("received_at".into(), json!(n.received_at.to_rfc3339()));
    doc.insert(
        "received_ms".into(),
        json!(n.received_at.timestamp_millis()),
    );
    let optional = [
        ("account_id", n.account_id.as_ref().map(|v| json!(v))),
        ("importance_score", n.importance_score.map(|v| json!(v))),
        ("triage_action", n.triage_action.as_ref().map(|v| json!(v))),
        ("triage_reason", n.triage_reason.as_ref().map(|v| json!(v))),
        ("scored_at", n.scored_at.map(|v| json!(v.to_rfc3339()))),
    ];
    for (field, value) in optional {
        if let Some(value) = value {
            doc.insert(field.into(), value);
        }
    }
    serde_json::Value::Object(doc)
}

/// Reads every legacy row.
pub(crate) fn read(config: &Config) -> Result<Vec<ImportDoc>> {
    read_for(config, LOCAL_WORKSPACE)
}

/// [`read`] with core notifications filed under `workspace` instead of the
/// fixed local key.
pub(crate) fn read_for(config: &Config, workspace: &str) -> Result<Vec<ImportDoc>> {
    with_connection(config, |conn| read_rows(conn, workspace))
}

fn read_rows(conn: &Connection, workspace: &str) -> Result<Vec<ImportDoc>> {
    let mut docs = Vec::new();
    if table_exists(conn, "integration_notifications")? {
        let mut stmt = conn
            .prepare(
                "SELECT id, provider, account_id, title, body, raw_payload,
                        importance_score, triage_action, triage_reason, status,
                        received_at, scored_at
                 FROM integration_notifications ORDER BY rowid",
            )
            .context("[notifications::import] prepare integration_notifications")?;
        let rows = stmt
            .query(())
            .context("[notifications::import] query integration_notifications")?;
        for notification in rows_to_notifications(rows)? {
            docs.push(ImportDoc {
                collection: LEGACY_NOTIFICATIONS,
                id: notification.id.clone(),
                doc: legacy_doc(&notification),
            });
        }
    }
    if table_exists(conn, "notification_settings")? {
        let mut stmt = conn
            .prepare(
                "SELECT provider, enabled, importance_threshold, route_to_orchestrator
                 FROM notification_settings ORDER BY rowid",
            )
            .context("[notifications::import] prepare notification_settings")?;
        let rows = stmt.query_map([], |row| {
            Ok(ImportDoc {
                collection: LEGACY_SETTINGS,
                id: row.get(0)?,
                doc: json!({
                    "enabled": row.get::<_, i64>(1)? != 0,
                    "importance_threshold": row.get::<_, f64>(2)?,
                    "route_to_orchestrator": row.get::<_, i64>(3)? != 0,
                }),
            })
        })?;
        for row in rows {
            docs.push(row.context("[notifications::import] decode a notification_settings row")?);
        }
    }
    if table_exists(conn, "core_notifications")? {
        let mut stmt = conn
            .prepare(
                "SELECT id, payload, timestamp_ms, read, created_at
                 FROM core_notifications ORDER BY rowid",
            )
            .context("[notifications::import] prepare core_notifications")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            Ok(ImportDoc {
                collection: LEGACY_CORE,
                id: legacy_core_id(workspace, &id),
                doc: json!({
                    "workspace": workspace,
                    "payload": row.get::<_, String>(1)?,
                    "timestamp_ms": row.get::<_, i64>(2)?,
                    "read": row.get::<_, i64>(3)? != 0,
                    "created_at": row.get::<_, String>(4)?,
                }),
            })
        })?;
        for row in rows {
            docs.push(row.context("[notifications::import] decode a core_notifications row")?);
        }
    }
    Ok(docs)
}

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "store_import_tests.rs"]
mod tests;
