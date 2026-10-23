//! Reader of the legacy `approval.db`, which the startup history importer
//! (`native/history_import.rs`) feeds to the module.
//!
//! `pending_approvals` rows become [`LegacyApproval`]s (the module keeps them as
//! sanitized, inert history through `ImportApprovalHistory2`) and
//! `flow_tool_trust` rows become [`LegacyTrust`]s (`GrantFlowTool2`). The file is
//! only ever opened here on a scratch copy of it, which the reader may migrate
//! and scrub; the real file is never modified.

use crate::storage::rusqlite::{params, Connection};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::storage::import::{table_exists, Snapshot};

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

/// One legacy request, as the retired store kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyApproval {
    pub(crate) request_id: String,
    pub(crate) tool_name: String,
    /// The agent the request ran for; `None` for the process's own.
    pub(crate) agent_id: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) decided_at: Option<DateTime<Utc>>,
    /// The stored decision (`approve_once`, `deny`, ...); `None` while undecided.
    pub(crate) decision: Option<String>,
    /// Whether the approved call ran.
    pub(crate) executed: bool,
}

/// One legacy "approve always for this flow" grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyTrust {
    pub(crate) flow_id: String,
    pub(crate) tool_name: String,
}

/// Everything the legacy file held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LegacyHistory {
    /// Oldest first.
    pub(crate) approvals: Vec<LegacyApproval>,
    pub(crate) trust: Vec<LegacyTrust>,
}

/// Reads the legacy file of `config`'s workspace from a scratch copy, or `None`
/// when the workspace has none. The file itself is never opened for writing.
pub(crate) fn read_legacy(config: &Config) -> Result<Option<LegacyHistory>> {
    let legacy = db_path(config);
    if !legacy.is_file() {
        return Ok(None);
    }
    let snapshot = Snapshot::of(&legacy, "approval/approval.db")
        .context("[approval::import] copy the legacy file for reading")?;
    let scratch = Config {
        workspace_dir: snapshot.workspace().to_path_buf(),
        ..config.clone()
    };
    with_connection(&scratch, read_rows).map(Some)
}

fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn read_rows(conn: &Connection) -> Result<LegacyHistory> {
    let mut history = LegacyHistory::default();
    if table_exists(conn, "pending_approvals")? {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, created_at, expires_at, decided_at, decision,
                        executed_at, agent_id
                 FROM pending_approvals ORDER BY rowid",
            )
            .context("[approval::import] prepare pending_approvals")?;
        let rows = stmt
            .query_map([], |row| {
                let text = |i: usize| row.get::<_, Option<String>>(i);
                let created: String = row.get(2)?;
                Ok(LegacyApproval {
                    request_id: row.get(0)?,
                    tool_name: row.get(1)?,
                    created_at: instant(&created).unwrap_or_else(Utc::now),
                    expires_at: text(3)?.as_deref().and_then(instant),
                    decided_at: text(4)?.as_deref().and_then(instant),
                    decision: text(5)?,
                    executed: text(6)?.is_some(),
                    agent_id: text(7)?.filter(|agent| !agent.is_empty()),
                })
            })
            .context("[approval::import] query pending_approvals")?;
        for row in rows {
            history
                .approvals
                .push(row.context("[approval::import] decode a pending_approvals row")?);
        }
    }
    if table_exists(conn, "flow_tool_trust")? {
        let mut stmt = conn
            .prepare("SELECT flow_id, tool_name FROM flow_tool_trust ORDER BY rowid")
            .context("[approval::import] prepare flow_tool_trust")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LegacyTrust {
                    flow_id: row.get(0)?,
                    tool_name: row.get(1)?,
                })
            })
            .context("[approval::import] query flow_tool_trust")?;
        for row in rows {
            history
                .trust
                .push(row.context("[approval::import] decode a flow_tool_trust row")?);
        }
    }
    Ok(history)
}

#[cfg(test)]
#[path = "store_import_tests.rs"]
mod tests;
