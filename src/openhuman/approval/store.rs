//! SQLite persistence for pending approval requests.
//!
//! Pending rows survive core restart so a queued approval is not lost
//! when the user quits before deciding. Each row carries the
//! `session_id` of the launch that queued it (informational only).
//! `list_pending` returns every undecided row regardless of session so
//! the UI can audit or dismiss orphans after restart, per the issue
//! #1339 acceptance criterion.
//!
//! Replay safety: a `decide` on an orphan row (process that queued it
//! is gone) updates the DB but cannot resume the parked future, so no
//! side effect can fire across processes.
//!
//! Durability safety: `expires_at` is enforced in the store. When a
//! pending row has already expired by the time the store is read again
//! after a restart, it is lazily transitioned into a terminal state so
//! stale rows stop showing up as actionable approvals forever.
//!
//! Follows the same `with_connection` shape as `notifications/store.rs`
//! and `cron/store.rs`: synchronous `rusqlite::Connection` opened per
//! call, schema applied idempotently.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, types::Type, Connection};

use crate::openhuman::config::Config;

use super::types::{ApprovalAuditEntry, ApprovalDecision, PendingApproval};

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

/// Transition any stale rows into a terminal state so they no longer
/// appear as actionable pending approvals after restart.
///
/// We currently reuse `deny` as the persisted terminal value to avoid
/// widening the externally visible approval decision enum before the
/// broader durable-audit work lands. This preserves the audit trail
/// (`decided_at` + `decision`) without leaving expired rows pending
/// forever.
pub fn expire_stale(config: &Config) -> Result<usize> {
    with_connection(config, |conn| expire_stale_with_now(conn, Utc::now()))
}

/// List all rows that are still awaiting user input, regardless of
/// which launch queued them. Orphan rows from prior sessions remain
/// visible until they are explicitly decided or expire.
pub fn list_pending(config: &Config) -> Result<Vec<PendingApproval>> {
    with_connection(config, |conn| {
        expire_stale_with_now(conn, Utc::now())?;

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
/// Returns `Ok(None)` if no row matched (already decided, expired, or
/// unknown id).
pub fn decide(
    config: &Config,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<Option<PendingApproval>> {
    with_connection(config, |conn| {
        expire_stale_with_now(conn, Utc::now())?;

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

/// List recently decided approval rows for durable audit views.
pub fn list_recent_decisions(config: &Config, limit: usize) -> Result<Vec<ApprovalAuditEntry>> {
    let limit = limit.clamp(1, 500);
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, action_summary, args_redacted,
                        session_id, created_at, expires_at, decided_at, decision
                 FROM pending_approvals
                 WHERE decided_at IS NOT NULL AND decision IS NOT NULL
                 ORDER BY decided_at DESC
                 LIMIT ?1",
            )
            .context("[approval::store] prepare list_recent_decisions")?;
        let rows = stmt
            .query_map(params![limit as i64], |row| Ok(row_to_audit_entry(row)))
            .context("[approval::store] query list_recent_decisions")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("[approval::store] audit row decode")??);
        }
        Ok(out)
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

fn expire_stale_with_now(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let now_rfc3339 = now.to_rfc3339();
    let deny = ApprovalDecision::Deny.as_str();
    let updated = conn
        .execute(
            "UPDATE pending_approvals
             SET decided_at = ?1, decision = ?2
             WHERE decided_at IS NULL
               AND expires_at IS NOT NULL
               AND strftime('%s', expires_at) <= strftime('%s', ?3)",
            params![now_rfc3339, deny, now_rfc3339],
        )
        .context("[approval::store] expire stale rows")?;
    Ok(updated)
}

fn row_to_audit_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalAuditEntry> {
    let args_str: String = row.get(3)?;
    let args_redacted: serde_json::Value = serde_json::from_str(&args_str)
        .unwrap_or_else(|_| serde_json::json!({ "_error": "args_redacted not valid JSON" }));
    let created_str: String = row.get(5)?;
    let expires_opt: Option<String> = row.get(6)?;
    let decided_str: String = row.get(7)?;
    let decision_str: String = row.get(8)?;
    let decision = ApprovalDecision::from_str(&decision_str).ok_or_else(|| {
        invalid_text_column(8, format!("unknown approval decision `{decision_str}`"))
    })?;
    Ok(ApprovalAuditEntry {
        request_id: row.get(0)?,
        tool_name: row.get(1)?,
        action_summary: row.get(2)?,
        args_redacted,
        session_id: row.get(4)?,
        created_at: parse_audit_rfc3339(5, &created_str)?,
        expires_at: expires_opt
            .as_deref()
            .map(|value| parse_audit_rfc3339(6, value))
            .transpose()?,
        decided_at: parse_audit_rfc3339(7, &decided_str)?,
        decision,
    })
}

fn parse_audit_rfc3339(column: usize, input: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

fn invalid_text_column(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn row_to_pending(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingApproval> {
    let args_str: String = row.get(3)?;
    let args_redacted = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
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
        sample_with_expiry(
            request_id,
            session_id,
            Some(Utc::now() + Duration::minutes(10)),
        )
    }

    fn sample_with_expiry(
        request_id: &str,
        session_id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> PendingApproval {
        PendingApproval {
            request_id: request_id.to_string(),
            tool_name: "composio".to_string(),
            action_summary: "send slack message (12 chars)".to_string(),
            args_redacted: json!({ "action": "execute", "tool_slug": "SLACK_SEND" }),
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            expires_at,
        }
    }

    fn fetch_decision_state(
        config: &Config,
        request_id: &str,
    ) -> Option<(Option<String>, Option<String>)> {
        with_connection(config, |conn| {
            let mut stmt = conn
                .prepare("SELECT decided_at, decision FROM pending_approvals WHERE request_id = ?1")
                .context("prepare raw decision lookup")?;
            let mut rows = stmt
                .query(params![request_id])
                .context("query raw decision lookup")?;
            if let Some(row) = rows.next().context("decision row next")? {
                let decided_at: Option<String> = row.get(0)?;
                let decision: Option<String> = row.get(1)?;
                Ok(Some((decided_at, decision)))
            } else {
                Ok(None)
            }
        })
        .unwrap()
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
        let remaining = list_pending(&config).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].request_id, "p3");
    }

    #[test]
    fn pending_row_survives_connection_close() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("survives", "sess-A")).unwrap();
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "survives");
    }

    #[test]
    fn list_pending_expires_stale_rows_before_returning() {
        let (config, _dir) = test_config();
        insert_pending(
            &config,
            &sample_with_expiry("expired", "sess-A", Some(Utc::now() - Duration::minutes(5))),
        )
        .unwrap();
        insert_pending(
            &config,
            &sample_with_expiry("active", "sess-A", Some(Utc::now() + Duration::minutes(5))),
        )
        .unwrap();

        let rows = list_pending(&config).unwrap();
        let ids: Vec<_> = rows.into_iter().map(|row| row.request_id).collect();
        assert_eq!(ids, vec!["active"]);

        let state = fetch_decision_state(&config, "expired").expect("expired row should persist");
        assert!(
            state.0.is_some(),
            "expired row should have decided_at recorded"
        );
        assert_eq!(state.1.as_deref(), Some("deny"));
    }

    #[test]
    fn decide_on_expired_row_returns_none_and_keeps_terminal_audit_state() {
        let (config, _dir) = test_config();
        insert_pending(
            &config,
            &sample_with_expiry("late", "sess-A", Some(Utc::now() - Duration::minutes(1))),
        )
        .unwrap();

        let decided = decide(&config, "late", ApprovalDecision::ApproveOnce).unwrap();
        assert!(
            decided.is_none(),
            "late approvals should no longer be actionable"
        );

        let state = fetch_decision_state(&config, "late").expect("row should remain for audit");
        assert!(state.0.is_some());
        assert_eq!(state.1.as_deref(), Some("deny"));
    }

    #[test]
    fn expire_stale_returns_number_of_rows_transitioned() {
        let (config, _dir) = test_config();
        insert_pending(
            &config,
            &sample_with_expiry("old-1", "sess-A", Some(Utc::now() - Duration::minutes(2))),
        )
        .unwrap();
        insert_pending(
            &config,
            &sample_with_expiry("old-2", "sess-B", Some(Utc::now() - Duration::minutes(1))),
        )
        .unwrap();
        insert_pending(
            &config,
            &sample_with_expiry("fresh", "sess-B", Some(Utc::now() + Duration::minutes(30))),
        )
        .unwrap();

        let expired = expire_stale(&config).unwrap();
        assert_eq!(expired, 2);

        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "fresh");
    }

    #[test]
    fn expire_stale_is_idempotent() {
        let (config, _dir) = test_config();
        insert_pending(
            &config,
            &sample_with_expiry("once", "sess-A", Some(Utc::now() - Duration::minutes(3))),
        )
        .unwrap();

        assert_eq!(expire_stale(&config).unwrap(), 1);
        assert_eq!(expire_stale(&config).unwrap(), 0);

        let state = fetch_decision_state(&config, "once").expect("row should remain recorded");
        assert!(state.0.is_some());
        assert_eq!(state.1.as_deref(), Some("deny"));
    }

    #[test]
    fn expire_stale_leaves_non_expiring_rows_pending() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample_with_expiry("no-ttl", "sess-A", None)).unwrap();

        assert_eq!(expire_stale(&config).unwrap(), 0);
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "no-ttl");

        let state = fetch_decision_state(&config, "no-ttl").expect("row should still exist");
        assert!(state.0.is_none());
        assert!(state.1.is_none());
    }

    #[test]
    fn list_recent_decisions_returns_durable_audit_rows() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("approved", "sess-A")).unwrap();
        insert_pending(&config, &sample("denied", "sess-B")).unwrap();
        decide(&config, "approved", ApprovalDecision::ApproveOnce).unwrap();
        decide(&config, "denied", ApprovalDecision::Deny).unwrap();

        let rows = list_recent_decisions(&config, 10).unwrap();

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| {
            row.request_id == "approved" && row.decision == ApprovalDecision::ApproveOnce
        }));
        assert!(rows
            .iter()
            .any(|row| row.request_id == "denied" && row.decision == ApprovalDecision::Deny));
        assert!(
            rows.iter().all(|row| !row.tool_name.is_empty()),
            "audit rows should retain tool metadata"
        );
    }

    #[test]
    fn list_recent_decisions_clamps_zero_limit_to_one() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("one", "sess-A")).unwrap();
        insert_pending(&config, &sample("two", "sess-A")).unwrap();
        decide(&config, "one", ApprovalDecision::ApproveOnce).unwrap();
        decide(&config, "two", ApprovalDecision::Deny).unwrap();

        let rows = list_recent_decisions(&config, 0).unwrap();

        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn list_recent_decisions_rejects_unknown_decision_values() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("corrupt-decision", "sess-A")).unwrap();
        with_connection(&config, |conn| {
            conn.execute(
                "UPDATE pending_approvals
                 SET decided_at = ?1, decision = ?2
                 WHERE request_id = ?3",
                params![Utc::now().to_rfc3339(), "maybe", "corrupt-decision"],
            )?;
            Ok(())
        })
        .unwrap();

        let err = list_recent_decisions(&config, 10).unwrap_err();

        assert!(
            err.to_string().contains("Invalid column type")
                || err.to_string().contains("unknown approval decision"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn list_recent_decisions_rejects_invalid_audit_timestamps() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("corrupt-time", "sess-A")).unwrap();
        with_connection(&config, |conn| {
            conn.execute(
                "UPDATE pending_approvals
                 SET decided_at = ?1, decision = ?2
                 WHERE request_id = ?3",
                params![
                    "not-a-date",
                    ApprovalDecision::Deny.as_str(),
                    "corrupt-time"
                ],
            )?;
            Ok(())
        })
        .unwrap();

        let err = list_recent_decisions(&config, 10).unwrap_err();

        assert!(
            err.to_string().contains("Invalid column type")
                || err.to_string().contains("premature end of input"),
            "unexpected error: {err}"
        );
    }
}
