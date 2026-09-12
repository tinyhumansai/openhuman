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

use crate::openhuman::config::Config;
use crate::openhuman::memory::safety::sanitize_text;

use super::types::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, PendingApproval,
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
    ] {
        if !have.contains(col) {
            conn.execute(ddl, params![])
                .with_context(|| format!("[approval::store] add column {col}"))?;
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
                 session_id, created_at, expires_at, source_context)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                pending.request_id,
                pending.tool_name,
                pending.action_summary,
                args,
                session_id,
                created,
                expires,
                source_context,
            ],
        )
        .context("[approval::store] insert pending row")?;
        Ok(())
    })
}

/// Record a save-time flow pre-authorization in the durable audit trail as a
/// born-decided row (`decided_at = created_at`, decision
/// `approve_always_for_flow`): it never appears in `list_pending` (which
/// filters `decided_at IS NULL`) but does surface in
/// `list_recent_decisions`, so Settings → Approval history shows exactly
/// when and for which tool the user granted blanket trust. The
/// `source_context` carries an empty `run_id` — no run existed yet — which
/// also keeps it invisible to `list_pending_for_flow_run`.
pub fn record_flow_preauthorization(
    config: &Config,
    flow_id: &str,
    tool_name: &str,
    session_id: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        let now = Utc::now().to_rfc3339();
        let source_context = serde_json::to_string(&ApprovalSourceContext::Flow {
            flow_id: flow_id.to_string(),
            run_id: String::new(),
            node_id: None,
        })
        .context("[approval::store] serialize preauthorization source_context")?;
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted,
                 session_id, created_at, expires_at, source_context,
                 decided_at, decision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?6, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                tool_name,
                "Pre-authorized for this flow when it was saved and enabled",
                "{}",
                session_id,
                now,
                source_context,
                ApprovalDecision::ApproveAlwaysForFlow.as_str(),
            ],
        )
        .context("[approval::store] insert preauthorization audit row")?;
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
                        session_id, created_at, expires_at, source_context
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
                        session_id, created_at, expires_at, source_context
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

/// Grant "approve always for this flow" trust to a `(flow_id, tool_name)`
/// pair — inserted when the user picks `ApproveAlwaysForFlow` on a
/// flow-origin park. `INSERT OR IGNORE` makes re-granting an already-trusted
/// pair a harmless no-op rather than a primary-key error.
pub fn insert_flow_trust(config: &Config, flow_id: &str, tool_name: &str) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO flow_tool_trust (flow_id, tool_name, created_at)
             VALUES (?1, ?2, ?3)",
            params![flow_id, tool_name, Utc::now().to_rfc3339()],
        )
        .context("[approval::store] insert_flow_trust")?;
        Ok(())
    })
}

/// List every `tool_name` currently holding "approve always for this flow"
/// trust for `flow_id`, ordered by name for stable output. Used by the
/// save-time pre-authorization manifest (`flows_approval_manifest`) to diff
/// "what the graph needs" against "what is already granted".
pub fn list_flow_trust(config: &Config, flow_id: &str) -> Result<Vec<String>> {
    with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT tool_name FROM flow_tool_trust
                 WHERE flow_id = ?1 ORDER BY tool_name",
            )
            .context("[approval::store] list_flow_trust prepare")?;
        let names = stmt
            .query_map(params![flow_id], |row| row.get::<_, String>(0))
            .context("[approval::store] list_flow_trust query")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("[approval::store] list_flow_trust rows")?;
        Ok(names)
    })
}

/// Delete flow trust rows for `flow_id`. With `tool_names: None` every grant
/// for the flow is removed (flow deletion cleanup); with `Some(names)` only
/// the named grants are revoked. Returns the number of rows removed. Deleting
/// a name that was never granted is a no-op, keeping the call idempotent.
pub fn delete_flow_trust(
    config: &Config,
    flow_id: &str,
    tool_names: Option<&[String]>,
) -> Result<usize> {
    with_connection(config, |conn| {
        let removed = match tool_names {
            None => conn
                .execute(
                    "DELETE FROM flow_tool_trust WHERE flow_id = ?1",
                    params![flow_id],
                )
                .context("[approval::store] delete_flow_trust all")?,
            Some(names) => {
                let mut removed = 0usize;
                for name in names {
                    removed += conn
                        .execute(
                            "DELETE FROM flow_tool_trust
                             WHERE flow_id = ?1 AND tool_name = ?2",
                            params![flow_id, name],
                        )
                        .context("[approval::store] delete_flow_trust named")?;
                }
                removed
            }
        };
        Ok(removed)
    })
}

/// Whether `(flow_id, tool_name)` was previously granted "approve always for
/// this flow" trust. Consulted by [`super::gate::ApprovalGate::intercept_audited`]
/// before parking a `Workflow`-origin tool call.
pub fn is_flow_tool_trusted(config: &Config, flow_id: &str, tool_name: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM flow_tool_trust WHERE flow_id = ?1 AND tool_name = ?2
                 )",
                params![flow_id, tool_name],
                |row| row.get(0),
            )
            .context("[approval::store] is_flow_tool_trusted")?;
        Ok(exists)
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
