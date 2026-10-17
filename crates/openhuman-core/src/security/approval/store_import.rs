//! One-shot import of the legacy `approval.db` tables into the document
//! layout (`store_documents.rs`).
//!
//! `pending_approvals` becomes one `approvals` document per request and
//! `flow_tool_trust` one `approval_flow_trust` document per grant. Documents
//! are shaped exactly as `store_documents.rs` writes them: timestamps in its
//! fixed-width form, `pending` as a boolean, `expires_ts` as the comparable
//! expiry key, optional columns left out when NULL. The legacy file is only
//! ever opened here, by the storage importer, on a scratch copy of it.

use crate::storage::rusqlite::{params, Connection};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use super::storage_schema::{LEGACY_APPROVALS, LEGACY_FLOW_TRUST};
use crate::config::Config;
use crate::storage::import::{table_exists, ImportDoc};

/// SQL schema of the legacy database, applied on every `with_connection` call.
///
/// `executed_at`, `execution_outcome`, and `execution_error` capture
/// the *after-action* audit row introduced for issue #2135 so a
/// reader can see both "the action was approved at X" and "the
/// action ran at Y with outcome Z" from the same table. Pre-existing
/// rows from older builds back-fill these as NULL — see
/// [`migrate_columns`] for the live-upgrade path.
const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS pending_approvals (
    request_id        TEXT PRIMARY KEY,
    tool_name         TEXT NOT NULL,
    action_summary    TEXT NOT NULL,
    args_redacted     TEXT NOT NULL,
    session_id        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    expires_at        TEXT,
    decided_at        TEXT,
    decision          TEXT,
    executed_at       TEXT,
    execution_outcome TEXT,
    execution_error   TEXT,
    source_context    TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_pending
    ON pending_approvals(decided_at);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_session
    ON pending_approvals(session_id);

-- Per-flow tool trust (flow-approval-surface, issue B-flows-approval PR2):
-- an `ApproveAlwaysForFlow` decision inserts a row here instead of the
-- global `autonomy.auto_approve` allowlist, so the grant is scoped to one
-- workflow's runs (including scheduled/triggered ones) rather than every
-- flow that happens to call the same tool.
CREATE TABLE IF NOT EXISTS flow_tool_trust (
    flow_id    TEXT NOT NULL,
    tool_name  TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (flow_id, tool_name)
);
";

/// Idempotently add the post-execution audit columns to an existing
/// `pending_approvals` table. `CREATE TABLE IF NOT EXISTS` above is
/// a no-op when the table already exists, so a DB created by an
/// older build keeps the v1 schema until this migration patches it.
///
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so we read
/// `PRAGMA table_info` and add missing columns one at a time.
fn migrate_columns(conn: &Connection) -> Result<()> {
    let mut have: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stmt = conn
        .prepare("PRAGMA table_info(pending_approvals)")
        .context("[approval::store] prepare table_info")?;
    let rows = stmt
        .query_map(params![], |row| row.get::<_, String>(1))
        .context("[approval::store] query table_info")?;
    for r in rows {
        have.insert(r.context("[approval::store] table_info row decode")?);
    }
    for (col, ddl) in [
        (
            "executed_at",
            "ALTER TABLE pending_approvals ADD COLUMN executed_at TEXT",
        ),
        (
            "execution_outcome",
            "ALTER TABLE pending_approvals ADD COLUMN execution_outcome TEXT",
        ),
        (
            "execution_error",
            "ALTER TABLE pending_approvals ADD COLUMN execution_error TEXT",
        ),
        (
            "source_context",
            "ALTER TABLE pending_approvals ADD COLUMN source_context TEXT",
        ),
        (
            "tool_call_id",
            "ALTER TABLE pending_approvals ADD COLUMN tool_call_id TEXT",
        ),
        (
            "agent_id",
            "ALTER TABLE pending_approvals ADD COLUMN agent_id TEXT",
        ),
    ] {
        if !have.contains(col) {
            // Two cores can open the same workspace during startup (for
            // example, a reconnecting desktop shell and its replacement).
            // Both may observe the old schema before either ALTER commits;
            // SQLite then reports a harmless duplicate-column race here.
            if let Err(error) = conn.execute(ddl, params![]) {
                if !error.to_string().contains("duplicate column name") {
                    return Err(error)
                        .with_context(|| format!("[approval::store] add column {col}"));
                }
            }
            tracing::info!(column = col, "[approval::store] migrated v1 schema");
        }
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_pending_approvals_agent
             ON pending_approvals(agent_id, decided_at)",
        params![],
    )
    .context("[approval::store] create agent index")?;
    Ok(())
}

/// Sentinel value written into the `session_id` column when scrubbing
/// legacy rows whose `session_id` may have stored a credential-shaped
/// value (an operator-supplied RPC bearer rather than a per-launch
/// UUID). Public so tests / future migrations can refer to it by
/// name.
pub const PRE_MIGRATION_SESSION_ID: &str = "pre-migration-redacted";

/// Idempotently scrub legacy `session_id` rows.
///
/// Earlier builds wrote the verbatim JSON-RPC bearer
/// (`OPENHUMAN_CORE_TOKEN`) into `pending_approvals.session_id`. The
/// column is retained for downgrade safety, but its stored value is
/// now a per-launch UUID with no credential material. This migration
/// overwrites any pre-existing value with [`PRE_MIGRATION_SESSION_ID`]
/// the first time a v1 DB is opened by a v2-aware build, then bumps
/// `PRAGMA user_version` to 1 so the rewrite never repeats.
fn migrate_session_id_scrub(conn: &Connection) -> Result<()> {
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", params![], |r| r.get(0))
        .context("[approval::store] read PRAGMA user_version")?;
    if user_version < 1 {
        let updated = conn
            .execute(
                "UPDATE pending_approvals SET session_id = ?1 WHERE session_id != ?1",
                params![PRE_MIGRATION_SESSION_ID],
            )
            .context("[approval::store] scrub legacy session_id")?;
        conn.execute_batch("PRAGMA user_version = 1;")
            .context("[approval::store] bump user_version to 1")?;
        if updated > 0 {
            tracing::info!(
                rows = updated,
                "[approval::store] scrubbed legacy session_id values from pending_approvals"
            );
        }
    }
    Ok(())
}

pub(crate) fn db_path(config: &Config) -> std::path::PathBuf {
    config.workspace_dir.join("approval").join("approval.db")
}

/// Open (and migrate) the approval DB, then call `f` with a live
/// connection. Mirrors `notifications/store.rs::with_connection`.
pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = db_path(config);

    tracing::trace!(
        path = %db_path.display(),
        "[approval::store] opening DB connection"
    );

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "[approval::store] failed to create dir {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path).with_context(|| {
        format!(
            "[approval::store] failed to open DB at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch(SCHEMA)
        .context("[approval::store] schema migration failed")?;
    migrate_columns(&conn)?;
    migrate_session_id_scrub(&conn)?;

    f(&conn)
}

/// Reads every legacy row, after bringing an old schema forward (on the
/// importer's scratch copy of the file).
pub(crate) fn read(config: &Config) -> Result<Vec<ImportDoc>> {
    with_connection(config, read_rows)
}

/// Fixed-width (nanosecond, `Z`) so the strings sort in time order: the v1
/// document layout the steps read.
fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// The comparable expiry key v1 stored: nanoseconds since the epoch, falling
/// back to milliseconds for instants beyond the i64-nanosecond range.
fn expiry_key(at: DateTime<Utc>) -> i64 {
    at.timestamp_nanos_opt()
        .unwrap_or_else(|| at.timestamp_millis().saturating_mul(1_000_000))
}

fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// `raw` in the document layout's fixed-width form, or unchanged when it is
/// not a timestamp (kept rather than dropped, so nothing is lost).
fn normalized(raw: &str) -> String {
    instant(raw).map_or_else(|| raw.to_string(), rfc3339)
}

fn read_rows(conn: &Connection) -> Result<Vec<ImportDoc>> {
    let mut docs = Vec::new();
    if table_exists(conn, "pending_approvals")? {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, action_summary, args_redacted, session_id,
                        created_at, expires_at, decided_at, decision, executed_at,
                        execution_outcome, execution_error, source_context,
                        tool_call_id, agent_id
                 FROM pending_approvals ORDER BY rowid",
            )
            .context("[approval::import] prepare pending_approvals")?;
        let rows = stmt
            .query_map([], |row| {
                let text = |i: usize| row.get::<_, Option<String>>(i);
                let mut doc = Map::new();
                let created = row.get::<_, String>(5)?;
                let expires = text(6)?;
                let decided = text(7)?;
                doc.insert("tool_name".into(), json!(row.get::<_, String>(1)?));
                doc.insert("action_summary".into(), json!(row.get::<_, String>(2)?));
                doc.insert("args_redacted".into(), json!(row.get::<_, String>(3)?));
                doc.insert("session_id".into(), json!(row.get::<_, String>(4)?));
                doc.insert("created_at".into(), json!(normalized(&created)));
                if let Some(expires) = &expires {
                    doc.insert("expires_at".into(), json!(normalized(expires)));
                    if let Some(at) = instant(expires) {
                        doc.insert("expires_ts".into(), json!(expiry_key(at)));
                    }
                }
                doc.insert("pending".into(), json!(decided.is_none()));
                for (field, value) in [
                    ("decided_at", decided.as_deref().map(normalized)),
                    ("decision", text(8)?),
                    ("executed_at", text(9)?.as_deref().map(normalized)),
                    ("execution_outcome", text(10)?),
                    ("execution_error", text(11)?),
                    ("source_context", text(12)?),
                    ("tool_call_id", text(13)?),
                    ("agent_id", text(14)?),
                ] {
                    if let Some(value) = value {
                        doc.insert(field.into(), json!(value));
                    }
                }
                Ok(ImportDoc {
                    collection: LEGACY_APPROVALS,
                    id: row.get(0)?,
                    doc: Value::Object(doc),
                })
            })
            .context("[approval::import] query pending_approvals")?;
        for row in rows {
            docs.push(row.context("[approval::import] decode a pending_approvals row")?);
        }
    }
    if table_exists(conn, "flow_tool_trust")? {
        let mut stmt = conn
            .prepare("SELECT flow_id, tool_name, created_at FROM flow_tool_trust ORDER BY rowid")
            .context("[approval::import] prepare flow_tool_trust")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("[approval::import] query flow_tool_trust")?;
        for row in rows {
            let (flow_id, tool_name, created) =
                row.context("[approval::import] decode a flow_tool_trust row")?;
            docs.push(ImportDoc {
                collection: LEGACY_FLOW_TRUST,
                id: format!("{}:{flow_id}/{tool_name}", flow_id.len()),
                doc: json!({
                    "flow_id": flow_id,
                    "tool_name": tool_name,
                    "created_at": normalized(&created),
                }),
            });
        }
    }
    Ok(docs)
}

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "store_import_tests.rs"]
mod tests;
