//! SQLite persistence for pending approval requests.
//!
//! Pending rows survive core restart so a queued approval is not lost
//! when the user quits before deciding. Each row carries the
//! `session_id` of the launch that queued it (informational —
//! `list_pending` returns every undecided row regardless of session
//! so the UI can audit / dismiss orphans after restart, per the
//! issue #1339 acceptance criterion).
//!
//! Replay safety: a `decide` on an orphan row (process that queued it
//! is gone) updates the DB but cannot resume the parked future — no
//! side effect can fire across processes. `purge_session` is a
//! best-effort cleanup helper kept for an explicit RPC in a follow-up.
//!
//! Follows the same `with_connection` shape as `notifications/store.rs`
//! and `cron/store.rs` — synchronous `rusqlite::Connection` opened per
//! call, schema applied idempotently.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::openhuman::config::Config;

use super::types::{ApprovalDecision, PendingApproval};

/// SQL schema applied on every `with_connection` call.
const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS pending_approvals (
    request_id      TEXT PRIMARY KEY,
    tool_name       TEXT NOT NULL,
    action_summary  TEXT NOT NULL,
    args_redacted   TEXT NOT NULL,
    session_id      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    expires_at      TEXT,
    decided_at      TEXT,
    decision        TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_pending
    ON pending_approvals(decided_at);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_session
    ON pending_approvals(session_id);
";

/// Open (and migrate) the approval DB, then call `f` with a live
/// connection. Mirrors `notifications/store.rs::with_connection`.
fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("approval").join("approval.db");

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

    f(&conn)
}

/// Insert a pending row. Caller supplies the `request_id` and
/// `session_id` so the gate can correlate the parked future.
pub fn insert_pending(config: &Config, pending: &PendingApproval) -> Result<()> {
    with_connection(config, |conn| {
        let args = serde_json::to_string(&pending.args_redacted)
            .context("[approval::store] serialize args_redacted")?;
        let created = pending.created_at.to_rfc3339();
        let expires = pending.expires_at.map(|t| t.to_rfc3339());
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted,
                 session_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                pending.request_id,
                pending.tool_name,
                pending.action_summary,
                args,
                pending.session_id,
                created,
                expires,
            ],
        )
        .context("[approval::store] insert pending row")?;
        Ok(())
    })
}

/// List all rows with no `decided_at` (still awaiting user input)
/// regardless of which launch queued them. Orphan rows (the gate's
/// in-memory waiter has been dropped — process died between
/// `intercept` and the user's decision) stay visible so the UI can
/// audit / dismiss them after restart, satisfying the issue #1339
/// acceptance criterion "pending rows survive app restart".
///
/// `decide` on an orphan row updates the DB and returns the row but
/// the parked tool call is gone — no side effect ever fires, which
/// matches the security invariant.
pub fn list_pending(config: &Config) -> Result<Vec<PendingApproval>> {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, action_summary, args_redacted,
                        session_id, created_at, expires_at
                 FROM pending_approvals
                 WHERE decided_at IS NULL
                 ORDER BY created_at ASC",
            )
            .context("[approval::store] prepare list_pending")?;
        let rows = stmt
            .query_map(params![], |row| Ok(row_to_pending(row)))
            .context("[approval::store] query list_pending")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("[approval::store] row decode")??);
        }
        Ok(out)
    })
}

/// Mark a pending row as decided and return the now-decided row.
/// Returns `Ok(None)` if no row matched (already decided, expired,
/// or unknown id).
pub fn decide(
    config: &Config,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<Option<PendingApproval>> {
    with_connection(config, |conn| {
        let decision_str = decision.as_str();
        let now = Utc::now().to_rfc3339();
        let updated = conn
            .execute(
                "UPDATE pending_approvals
                 SET decided_at = ?1, decision = ?2
                 WHERE request_id = ?3 AND decided_at IS NULL",
                params![now, decision_str, request_id],
            )
            .context("[approval::store] update decided")?;
        if updated == 0 {
            return Ok(None);
        }
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, action_summary, args_redacted,
                        session_id, created_at, expires_at
                 FROM pending_approvals WHERE request_id = ?1",
            )
            .context("[approval::store] prepare select decided")?;
        let mut rows = stmt
            .query(params![request_id])
            .context("[approval::store] query decided row")?;
        if let Some(row) = rows.next().context("[approval::store] decided row next")? {
            Ok(Some(row_to_pending(row)?))
        } else {
            Ok(None)
        }
    })
}

/// Drop all rows owned by `session_id` — called when the gate detects
/// a session changeover so stale parked rows do not accumulate.
pub fn purge_session(config: &Config, session_id: &str) -> Result<usize> {
    with_connection(config, |conn| {
        let removed = conn
            .execute(
                "DELETE FROM pending_approvals
                 WHERE session_id = ?1 AND decided_at IS NULL",
                params![session_id],
            )
            .context("[approval::store] purge_session")?;
        Ok(removed)
    })
}

fn row_to_pending(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingApproval> {
    let args_str: String = row.get(3)?;
    let args_redacted: serde_json::Value = serde_json::from_str(&args_str)
        .unwrap_or_else(|_| serde_json::json!({ "_error": "args_redacted not valid JSON" }));
    let created_str: String = row.get(5)?;
    let expires_opt: Option<String> = row.get(6)?;
    Ok(PendingApproval {
        request_id: row.get(0)?,
        tool_name: row.get(1)?,
        action_summary: row.get(2)?,
        args_redacted,
        session_id: row.get(4)?,
        created_at: parse_rfc3339(&created_str),
        expires_at: expires_opt.as_deref().map(parse_rfc3339),
    })
}

fn parse_rfc3339(input: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(input)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::approval::types::{ApprovalDecision, PendingApproval};
    use chrono::Duration;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config() -> (Config, TempDir) {
        let dir = TempDir::new().unwrap();
        let config = Config {
            workspace_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        (config, dir)
    }

    fn sample(request_id: &str, session_id: &str) -> PendingApproval {
        PendingApproval {
            request_id: request_id.to_string(),
            tool_name: "composio".to_string(),
            action_summary: "send slack message (12 chars)".to_string(),
            args_redacted: json!({ "action": "execute", "tool_slug": "SLACK_SEND" }),
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            expires_at: Some(Utc::now() + Duration::minutes(10)),
        }
    }

    #[test]
    fn insert_then_list_returns_pending_row() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-1", "sess-A")).unwrap();
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "req-1");
        assert_eq!(rows[0].tool_name, "composio");
    }

    #[test]
    fn list_pending_returns_rows_from_every_session() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("a", "sess-A")).unwrap();
        insert_pending(&config, &sample("b", "sess-B")).unwrap();
        let rows = list_pending(&config).unwrap();
        assert_eq!(
            rows.len(),
            2,
            "orphan rows from other sessions must remain visible"
        );
    }

    #[test]
    fn decide_marks_row_and_excludes_from_pending_list() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-9", "sess-A")).unwrap();
        let decided = decide(&config, "req-9", ApprovalDecision::ApproveOnce)
            .unwrap()
            .expect("decided row");
        assert_eq!(decided.request_id, "req-9");
        let rows = list_pending(&config).unwrap();
        assert!(rows.is_empty(), "decided rows should not appear in pending");
    }

    #[test]
    fn decide_second_time_returns_none() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("dupe", "sess-A")).unwrap();
        decide(&config, "dupe", ApprovalDecision::Deny).unwrap();
        let again = decide(&config, "dupe", ApprovalDecision::ApproveOnce).unwrap();
        assert!(again.is_none(), "second decide should be a no-op");
    }

    #[test]
    fn decide_unknown_id_is_noop() {
        let (config, _dir) = test_config();
        let res = decide(&config, "never-existed", ApprovalDecision::Deny).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn purge_session_removes_only_undecided_rows_for_session() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("p1", "sess-A")).unwrap();
        insert_pending(&config, &sample("p2", "sess-A")).unwrap();
        insert_pending(&config, &sample("p3", "sess-B")).unwrap();
        decide(&config, "p2", ApprovalDecision::ApproveOnce).unwrap();
        let removed = purge_session(&config, "sess-A").unwrap();
        assert_eq!(removed, 1, "only undecided sess-A row should be purged");
        // p2 stays because it is decided; sess-B untouched.
        let remaining = list_pending(&config).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].request_id, "p3");
    }

    #[test]
    fn pending_row_survives_connection_close() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("survives", "sess-A")).unwrap();
        // Each `with_connection` opens a fresh handle — re-reading
        // proves the row persisted to disk (acceptance criterion:
        // pending rows survive app restart).
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "survives");
    }
}
