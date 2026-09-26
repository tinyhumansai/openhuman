//! SQLite persistence for pending approval requests.
//!
//! Pending rows survive core restart so a queued approval is not lost
//! when the user quits before deciding. Each row carries a per-launch
//! UUID in the internal `session_id` column for correlation only; the
//! value is never re-exposed through [`PendingApproval`] /
//! [`ApprovalAuditEntry`] (a previous schema stored a credential-shaped
//! value here, see the migration in [`with_connection`]).
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

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::memory::safety::sanitize_text;

use super::types::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, PendingApproval,
};

// Flow pre-authorization + per-flow tool trust persistence, split out to keep
// this file under the repo's per-file line budget — see that module's doc.
#[path = "store_flow_trust.rs"]
mod store_flow_trust;
pub use store_flow_trust::{
    delete_flow_trust, insert_flow_trust, is_flow_tool_trusted, list_flow_trust,
    record_flow_preauthorization,
};

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
    migrate_session_id_scrub(&conn)?;

    f(&conn)
}

/// Insert a pending approval row. `session_id` is the per-launch UUID
/// the gate hands in — it is written into the durable column for
/// internal correlation only and is never re-exposed on
/// [`PendingApproval`] (see that type's doc-comment).
pub fn insert_pending(config: &Config, pending: &PendingApproval, session_id: &str) -> Result<()> {
    with_connection(config, |conn| {
        let args = serde_json::to_string(&pending.args_redacted)
            .context("[approval::store] serialize args_redacted")?;
        let created = pending.created_at.to_rfc3339();
        let expires = pending.expires_at.map(|t| t.to_rfc3339());
        let source_context = pending
            .source_context
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("[approval::store] serialize source_context")?;
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted,
                 session_id, created_at, expires_at, source_context, tool_call_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                pending.request_id,
                pending.tool_name,
                pending.action_summary,
                args,
                session_id,
                created,
                expires,
                source_context,
                pending.tool_call_id,
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
    with_connection(config, |conn| {
        Ok(expire_stale_with_now(conn, Utc::now())?.len())
    })
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
                        session_id, created_at, expires_at, source_context, tool_call_id
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
                        session_id, created_at, expires_at, source_context, tool_call_id
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

/// Filter [`list_pending`] down to the rows correlated with a specific flow
/// run (`source_context == Flow { flow_id, run_id, .. }`). The table has no
/// dedicated index for this — pending rows are always few (parked, awaiting
/// a live decision), so a full scan + JSON-decode filter in Rust is simpler
/// than a JSON1 SQL predicate and avoids a SQLite extension dependency.
pub fn list_pending_for_flow_run(
    config: &Config,
    flow_id: &str,
    run_id: &str,
) -> Result<Vec<PendingApproval>> {
    let all = list_pending(config)?;
    Ok(all
        .into_iter()
        .filter(|row| {
            matches!(
                &row.source_context,
                Some(ApprovalSourceContext::Flow { flow_id: f, run_id: r, .. })
                    if f == flow_id && r == run_id
            )
        })
        .collect())
}

/// Lazily transition every stale (past-`expires_at`, undecided) row into a
/// terminal `Deny` state and return the rows that were transitioned.
///
/// Fetches the about-to-expire rows BEFORE the `UPDATE` (their non-decision
/// columns are immutable at that point) so the caller can publish a
/// `DomainEvent::ApprovalDecided { resolution: "expired" }` per row — a sweep
/// runs with no live `ApprovalGate` in scope (`list_pending`/`decide` are
/// called through the store, not the gate), so this is the only place that
/// observes an expiry and must be the one to tell the web channel a parked
/// card is now stale.
fn expire_stale_with_now(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<PendingApproval>> {
    let now_rfc3339 = now.to_rfc3339();
    let deny = ApprovalDecision::Deny.as_str();

    let mut about_to_expire: Vec<PendingApproval> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, tool_name, action_summary, args_redacted,
                        session_id, created_at, expires_at, source_context, tool_call_id
                 FROM pending_approvals
                 WHERE decided_at IS NULL
                   AND expires_at IS NOT NULL
                   AND strftime('%s', expires_at) <= strftime('%s', ?1)",
            )
            .context("[approval::store] prepare expire_stale select")?;
        let rows = stmt
            .query_map(params![now_rfc3339], |row| Ok(row_to_pending(row)))
            .context("[approval::store] query expire_stale select")?;
        for r in rows {
            about_to_expire.push(r.context("[approval::store] expire_stale row decode")??);
        }
    }

    if about_to_expire.is_empty() {
        return Ok(about_to_expire);
    }

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
    tracing::debug!(
        rows = updated,
        "[approval::store] lazily expired stale pending_approvals rows"
    );
    for row in &about_to_expire {
        BUS.publish(DomainEvent::ApprovalDecided {
            request_id: row.request_id.clone(),
            tool_name: row.tool_name.clone(),
            decision: deny.to_string(),
            thread_id: None,
            client_id: None,
            tool_call_id: row.tool_call_id.clone(),
            resolution: Some("expired".to_string()),
        });
    }
    Ok(about_to_expire)
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
    // Note: column index 4 (`session_id`) is read on the SELECT but
    // intentionally not surfaced — see `ApprovalAuditEntry` doc-comment.
    Ok(ApprovalAuditEntry {
        request_id: row.get(0)?,
        tool_name: row.get(1)?,
        action_summary: row.get(2)?,
        args_redacted,
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
    // Column 7 (`source_context`) is absent on rows written before this
    // migration and on every plain chat-routed park — tolerate both a
    // missing column read error and a NULL value as "no context" rather
    // than failing the whole row decode (older SELECTs on a freshly
    // migrated DB may race the column add on some SQLite builds).
    let source_context_str: Option<String> = row.get(7).unwrap_or(None);
    let source_context = source_context_str.as_deref().and_then(|raw| {
        serde_json::from_str::<ApprovalSourceContext>(raw)
            .map_err(|err| {
                tracing::warn!(
                    error = %err,
                    "[approval::store] failed to decode source_context JSON — treating as absent"
                );
                err
            })
            .ok()
    });
    // Column 8 (`tool_call_id`) is likewise absent on rows written before
    // this field existed — tolerate a missing-column read error as `None`.
    let tool_call_id: Option<String> = row.get(8).unwrap_or(None);

    // Note: column index 4 (`session_id`) is read on the SELECT but
    // intentionally not surfaced — see `PendingApproval` doc-comment.
    Ok(PendingApproval {
        request_id: row.get(0)?,
        tool_name: row.get(1)?,
        action_summary: row.get(2)?,
        args_redacted,
        created_at: parse_rfc3339(&created_str),
        expires_at: expires_opt.as_deref().map(parse_rfc3339),
        source_context,
        tool_call_id,
    })
}

fn parse_rfc3339(input: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(input)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
