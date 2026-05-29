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
use crate::openhuman::memory_store::safety::sanitize_text;

use super::types::{ApprovalAuditEntry, ApprovalDecision, ExecutionOutcome, PendingApproval};

/// SQL schema applied on every `with_connection` call.
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
    execution_error   TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_pending
    ON pending_approvals(decided_at);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_session
    ON pending_approvals(session_id);
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
    ] {
        if !have.contains(col) {
            conn.execute(ddl, params![])
                .with_context(|| format!("[approval::store] add column {col}"))?;
            tracing::info!(column = col, "[approval::store] migrated v1 schema");
        }
    }
    Ok(())
}

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
    migrate_columns(&conn)?;

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

/// Look up the persisted decision for a request_id without mutating
/// state. Returns `Ok(None)` when the row doesn't exist or hasn't
/// been decided yet. Used to resolve gate-timeout vs decide races
/// where the TTL elapses concurrently with a committed approval
/// (CodeRabbit review on PR #2367).
pub fn get_decision(config: &Config, request_id: &str) -> Result<Option<ApprovalDecision>> {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT decision FROM pending_approvals
                 WHERE request_id = ?1 AND decided_at IS NOT NULL",
            )
            .context("[approval::store] prepare get_decision")?;
        let mut rows = stmt
            .query(params![request_id])
            .context("[approval::store] query get_decision")?;
        if let Some(row) = rows.next().context("[approval::store] get_decision next")? {
            let raw: String = row
                .get(0)
                .context("[approval::store] get_decision decode")?;
            Ok(ApprovalDecision::from_str(&raw))
        } else {
            Ok(None)
        }
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

/// Persist the terminal status of a tool call the gate previously
/// allowed.
///
/// Writes `executed_at = now`, `execution_outcome`, and an optional
/// short error string back onto the original `pending_approvals`
/// row. Returns `Ok(true)` when the row was found and updated,
/// `Ok(false)` when no matching row exists (gate not installed, or
/// a stray `record_execution` for an id that was never persisted) —
/// the latter is a no-op so callers can fire it unconditionally
/// without branching on `Option<request_id>`.
///
/// **Invariant:** only call this AFTER `decide(..., ApproveOnce |
/// ApproveAlwaysForTool)` has succeeded — otherwise the row will
/// show an `executed_at` without a `decided_at`, which is nonsense.
/// The gate enforces this by only handing out a request_id when the
/// intercepted call was allowed.
pub fn record_execution(
    config: &Config,
    request_id: &str,
    outcome: ExecutionOutcome,
    error: Option<&str>,
) -> Result<bool> {
    with_connection(config, |conn| {
        let now = Utc::now().to_rfc3339();
        // Sanitize before truncation so the durable audit row can't
        // leak bearer tokens, API keys, private-key blocks, OAuth
        // params, emails, or other PII the upstream tool might have
        // echoed back into its error message (PR #2367 review).
        // Truncate-first would split a secret mid-string and dodge
        // the redaction regexes — sanitize, then cap. Cap is 512
        // chars inclusive of the ellipsis marker; the agent already
        // sees the full error in its own tool-result envelope so
        // nothing observable depends on the stored copy.
        let trimmed_error = error.map(|raw| {
            let sanitized = sanitize_text(raw).value;
            if sanitized.chars().count() > 512 {
                let head: String = sanitized.chars().take(511).collect();
                format!("{head}…")
            } else {
                sanitized
            }
        });
        // `executed_at IS NULL` makes the terminal audit row
        // immutable — the first `record_execution` call wins, and a
        // late retry/cleanup path can't silently rewrite the original
        // outcome (CodeRabbit review on #2367). `decided_at IS NOT
        // NULL` keeps the monotonic invariant (no "executed before
        // approved" rows).
        let updated = conn
            .execute(
                "UPDATE pending_approvals
                 SET executed_at = ?1,
                     execution_outcome = ?2,
                     execution_error = ?3
                 WHERE request_id = ?4
                   AND decided_at IS NOT NULL
                   AND executed_at IS NULL",
                params![now, outcome.as_str(), trimmed_error, request_id],
            )
            .context("[approval::store] record_execution update")?;
        Ok(updated > 0)
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
    fn get_decision_returns_none_until_decided_then_persisted_value() {
        // PR #2367 review: timeout-vs-decide race resolution in the
        // gate calls `get_decision` after a denied UPDATE no-ops.
        // Undecided rows and unknown ids must both return `None`,
        // and decided rows must round-trip the persisted decision.
        let (config, _dir) = test_config();
        assert!(get_decision(&config, "missing").unwrap().is_none());
        insert_pending(&config, &sample("race", "sess-A")).unwrap();
        assert!(
            get_decision(&config, "race").unwrap().is_none(),
            "undecided row reports no decision"
        );
        decide(&config, "race", ApprovalDecision::ApproveOnce).unwrap();
        assert_eq!(
            get_decision(&config, "race").unwrap(),
            Some(ApprovalDecision::ApproveOnce)
        );
    }

    #[test]
    fn pending_row_survives_connection_close() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("survives", "sess-A")).unwrap();
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "survives");
    }

    // ── record_execution / column-migration tests (#2135) ──────────

    fn read_execution_row(
        config: &Config,
        request_id: &str,
    ) -> (Option<String>, Option<String>, Option<String>) {
        with_connection(config, |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT executed_at, execution_outcome, execution_error
                     FROM pending_approvals WHERE request_id = ?1",
                )
                .unwrap();
            let mut rows = stmt.query(params![request_id]).unwrap();
            let row = rows.next().unwrap().expect("row exists");
            Ok((
                row.get::<_, Option<String>>(0).unwrap(),
                row.get::<_, Option<String>>(1).unwrap(),
                row.get::<_, Option<String>>(2).unwrap(),
            ))
        })
        .unwrap()
    }

    #[test]
    fn record_execution_writes_terminal_audit_row_after_decide() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-exec", "sess-A")).unwrap();
        // Before decide, record_execution must not patch the row —
        // a decided_at IS NOT NULL guard keeps the audit trail
        // monotonic (no "executed before approved").
        let early = record_execution(&config, "req-exec", ExecutionOutcome::Success, None).unwrap();
        assert!(!early, "record_execution before decide must be a no-op");
        let (exec_at, _, _) = read_execution_row(&config, "req-exec");
        assert!(exec_at.is_none());

        decide(&config, "req-exec", ApprovalDecision::ApproveOnce).unwrap();
        let ok = record_execution(&config, "req-exec", ExecutionOutcome::Success, None).unwrap();
        assert!(ok, "record_execution after decide must update the row");
        let (exec_at, outcome, error) = read_execution_row(&config, "req-exec");
        assert!(exec_at.is_some());
        assert_eq!(outcome.as_deref(), Some("success"));
        assert!(error.is_none());
    }

    #[test]
    fn record_execution_persists_outcome_and_redacted_error() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-fail", "sess-A")).unwrap();
        decide(&config, "req-fail", ApprovalDecision::ApproveOnce).unwrap();

        record_execution(
            &config,
            "req-fail",
            ExecutionOutcome::Failure,
            Some("backend returned 500"),
        )
        .unwrap();

        let (_, outcome, error) = read_execution_row(&config, "req-fail");
        assert_eq!(outcome.as_deref(), Some("failure"));
        assert_eq!(error.as_deref(), Some("backend returned 500"));
    }

    #[test]
    fn record_execution_caps_long_error_messages() {
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-long", "sess-A")).unwrap();
        decide(&config, "req-long", ApprovalDecision::ApproveOnce).unwrap();

        let huge = "x".repeat(2_000);
        record_execution(&config, "req-long", ExecutionOutcome::Failure, Some(&huge)).unwrap();

        let (_, _, error) = read_execution_row(&config, "req-long");
        let stored = error.expect("error stored");
        // 512-char cap is inclusive of the ellipsis marker
        // (CodeRabbit review on #2367) — anything longer would let
        // upstream crash dumps slowly fill the audit log.
        assert_eq!(
            stored.chars().count(),
            512,
            "truncated value must be exactly 512 chars (incl. ellipsis): {} chars",
            stored.chars().count()
        );
        assert!(stored.ends_with('…'));
    }

    #[test]
    fn record_execution_redacts_secrets_in_error_message() {
        // PR #2367 review: upstream tool errors regularly echo back
        // the offending request including auth headers. The audit
        // row must persist the sanitized form so a leaked bearer
        // or API key never lands in the durable log.
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-secret", "sess-A")).unwrap();
        decide(&config, "req-secret", ApprovalDecision::ApproveOnce).unwrap();

        let raw = "upstream 401: Authorization: Bearer sk-live-abcdef1234567890abcdef1234567890 \
             returned by sk-proj-FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE";
        record_execution(&config, "req-secret", ExecutionOutcome::Failure, Some(raw)).unwrap();

        let (_, _, error) = read_execution_row(&config, "req-secret");
        let stored = error.expect("error stored");
        assert!(
            !stored.contains("sk-live-abcdef1234567890abcdef1234567890"),
            "raw bearer token must not be persisted: {stored}"
        );
        assert!(
            !stored.contains("sk-proj-FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE"),
            "raw provider key must not be persisted: {stored}"
        );
        assert!(
            stored.contains("[REDACTED]"),
            "sanitizer must leave a redaction marker so audit reviewers see something was scrubbed: {stored}"
        );
    }

    #[test]
    fn record_execution_is_idempotent_after_first_terminal_report_wins() {
        // CodeRabbit review on #2367: a late retry / cleanup path
        // must NOT rewrite the original audit row. The first
        // `record_execution` call wins; subsequent calls return
        // `false` and leave the row unchanged.
        let (config, _dir) = test_config();
        insert_pending(&config, &sample("req-idem", "sess-A")).unwrap();
        decide(&config, "req-idem", ApprovalDecision::ApproveOnce).unwrap();

        // First report: succeeds, row gets stamped.
        let first = record_execution(
            &config,
            "req-idem",
            ExecutionOutcome::Success,
            Some("ok-first"),
        )
        .unwrap();
        assert!(first);
        let (exec_at_1, outcome_1, error_1) = read_execution_row(&config, "req-idem");
        assert!(exec_at_1.is_some());
        assert_eq!(outcome_1.as_deref(), Some("success"));
        assert_eq!(error_1.as_deref(), Some("ok-first"));

        // Second report (e.g. a late retry that finally noticed the
        // outcome) must be a no-op and must NOT change the stored
        // outcome or timestamp.
        let second = record_execution(
            &config,
            "req-idem",
            ExecutionOutcome::Failure,
            Some("late-failure-noise"),
        )
        .unwrap();
        assert!(
            !second,
            "second record_execution must report no row updated"
        );

        let (exec_at_2, outcome_2, error_2) = read_execution_row(&config, "req-idem");
        assert_eq!(exec_at_2, exec_at_1, "executed_at must not change");
        assert_eq!(outcome_2.as_deref(), Some("success"));
        assert_eq!(error_2.as_deref(), Some("ok-first"));
    }

    #[test]
    fn record_execution_unknown_id_is_safe_noop() {
        let (config, _dir) = test_config();
        let ok = record_execution(&config, "never-here", ExecutionOutcome::Success, None).unwrap();
        assert!(!ok, "unknown id must report no row updated");
    }

    #[test]
    fn migrate_columns_is_idempotent_on_v1_databases() {
        // Simulate an older build by creating the v1 table shape
        // manually (no executed_at / execution_outcome / execution_error)
        // then opening the store via with_connection — the migration
        // must add the missing columns without losing existing rows.
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().to_path_buf();
        let db_path = workspace.join("approval").join("approval.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE pending_approvals (
                    request_id      TEXT PRIMARY KEY,
                    tool_name       TEXT NOT NULL,
                    action_summary  TEXT NOT NULL,
                    args_redacted   TEXT NOT NULL,
                    session_id      TEXT NOT NULL,
                    created_at      TEXT NOT NULL,
                    expires_at      TEXT,
                    decided_at      TEXT,
                    decision        TEXT
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pending_approvals
                    (request_id, tool_name, action_summary, args_redacted,
                     session_id, created_at)
                 VALUES ('legacy', 'composio', 'legacy row', '{}', 'sess-X', ?1)",
                params![Utc::now().to_rfc3339()],
            )
            .unwrap();
        }
        let config = Config {
            workspace_dir: workspace,
            ..Config::default()
        };
        // First open triggers the migration; existing row survives.
        let rows = list_pending(&config).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "legacy");
        // After migration, record_execution must work end-to-end.
        decide(&config, "legacy", ApprovalDecision::ApproveOnce).unwrap();
        assert!(record_execution(&config, "legacy", ExecutionOutcome::Success, None).unwrap());
        // Second open must be a no-op (migration is idempotent).
        let rows = list_pending(&config).unwrap();
        assert!(rows.is_empty(), "decided rows should not appear in pending");
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
