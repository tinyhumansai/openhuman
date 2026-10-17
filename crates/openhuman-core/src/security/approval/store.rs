//! Approval requests: pending rows, decisions and the after-action audit trail.
//!
//! Pending rows survive core restart so a queued approval is not lost
//! when the user quits before deciding. Each row carries a per-launch
//! UUID in the internal `session_id` field for correlation only; the
//! value is never re-exposed through [`PendingApproval`] /
//! [`ApprovalAuditEntry`] (a previous schema stored a credential-shaped
//! value here, see the migration in `store_import.rs`).
//! `list_pending` returns every undecided row regardless of session so
//! the UI can audit or dismiss orphans after restart, per the issue
//! #1339 acceptance criterion.
//!
//! Replay safety: a `decide` on an orphan row (process that queued it
//! is gone) updates the store but cannot resume the parked future, so no
//! side effect can fire across processes.
//!
//! Durability safety: `expires_at` is enforced in the store. When a
//! pending row has already expired by the time the store is read again
//! after a restart, it is lazily transitioned into a terminal state so
//! stale rows stop showing up as actionable approvals forever.
//!
//! Every function here uses `store_documents`: the operations on the
//! `tinystoragedrivers` document port, under the current call's storage
//! scope (the acting agent; `local` on a single-user host; refused in SaaS
//! mode without one). The `approval/approval.db` older installs wrote is read
//! once, read-only, by the storage importer (`store_import.rs`).

use anyhow::Result;
use chrono::Utc;

use super::store_documents::Docs;
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

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

pub use super::store_import::PRE_MIGRATION_SESSION_ID;

/// Insert a pending approval row. `session_id` is the per-launch UUID
/// the gate hands in — it is written into the durable field for
/// internal correlation only and is never re-exposed on
/// [`PendingApproval`] (see that type's doc-comment).
pub fn insert_pending(config: &Config, pending: &PendingApproval, session_id: &str) -> Result<()> {
    super::store_documents::current(config)?.insert_pending(pending, session_id)
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
    let expired = super::store_documents::current(config)?.expire_stale(Utc::now())?;
    publish_expired(&expired);
    Ok(expired.len())
}

/// List all rows that are still awaiting user input, regardless of
/// which launch queued them. Orphan rows from prior sessions remain
/// visible until they are explicitly decided or expire.
pub fn list_pending(config: &Config) -> Result<Vec<PendingApproval>> {
    let docs = super::store_documents::current(config)?;
    publish_expired(&docs.expire_stale(Utc::now())?);
    docs.list_pending()
}

/// Look up the persisted decision for a request_id without mutating
/// state. Returns `Ok(None)` when the row doesn't exist or hasn't
/// been decided yet. Used to resolve gate-timeout vs decide races
/// where the TTL elapses concurrently with a committed approval
/// (CodeRabbit review on PR #2367).
pub fn get_decision(config: &Config, request_id: &str) -> Result<Option<ApprovalDecision>> {
    super::store_documents::current(config)?.get_decision(request_id)
}

/// The document store of the current call, for a caller that decides later
/// from outside this call's task scope (a `Drop`), where the acting agent is
/// no longer installed. A scope that cannot be resolved is kept as an error,
/// so the later decision fails instead of acting under another scope.
pub(super) fn capture_docs(config: &Config) -> Result<Docs, String> {
    super::store_documents::current(config).map_err(|error| error.to_string())
}

/// [`decide`] in an already-resolved store (the one a request was parked in).
pub(super) fn decide_in(
    docs: &Docs,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<Option<PendingApproval>> {
    publish_expired(&docs.expire_stale(Utc::now())?);
    docs.decide(request_id, decision)
}

/// [`decide`] against a store captured by [`capture_docs`].
pub(super) fn decide_captured(
    captured: &Result<Docs, String>,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<Option<PendingApproval>> {
    match captured {
        Ok(docs) => decide_in(docs, request_id, decision),
        Err(error) => Err(anyhow::anyhow!(
            "[approval::store] storage scope was unresolved when the request parked: {error}"
        )),
    }
}

/// Mark a pending row as decided and return the now-decided row.
/// Returns `Ok(None)` if no row matched (already decided, expired, or
/// unknown id).
pub fn decide(
    config: &Config,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<Option<PendingApproval>> {
    let docs = super::store_documents::current(config)?;
    publish_expired(&docs.expire_stale(Utc::now())?);
    docs.decide(request_id, decision)
}

/// Persist the terminal status of a tool call the gate previously
/// allowed.
///
/// Writes `executed_at = now`, `execution_outcome`, and an optional
/// short error string back onto the original request. Returns `Ok(true)`
/// when the row was found and updated, `Ok(false)` when no matching row
/// exists (gate not installed, or a stray `record_execution` for an id that
/// was never persisted) — the latter is a no-op so callers can fire it
/// unconditionally without branching on `Option<request_id>`.
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
    super::store_documents::current(config)?.record_execution(request_id, outcome, error)
}

/// List recently decided approval rows for durable audit views.
pub fn list_recent_decisions(config: &Config, limit: usize) -> Result<Vec<ApprovalAuditEntry>> {
    let limit = limit.clamp(1, 500);
    super::store_documents::current(config)?.list_recent_decisions(limit)
}

/// Drop all undecided rows owned by `session_id` — called when the gate
/// detects a session changeover so stale parked rows do not accumulate.
pub fn purge_session(config: &Config, session_id: &str) -> Result<usize> {
    super::store_documents::current(config)?.purge_session(session_id)
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

/// Tells the web channel each expired row's parked card is now stale — a
/// sweep runs with no live `ApprovalGate` in scope, so the store is the only
/// place that observes an expiry.
fn publish_expired(expired: &[PendingApproval]) {
    for row in expired {
        BUS.publish(DomainEvent::ApprovalDecided {
            request_id: row.request_id.clone(),
            tool_name: row.tool_name.clone(),
            decision: ApprovalDecision::Deny.as_str().to_string(),
            thread_id: None,
            client_id: None,
            tool_call_id: row.tool_call_id.clone(),
            resolution: Some("expired".to_string()),
            agent_id: row.agent_id.clone(),
        });
    }
}

#[path = "store_agent.rs"]
mod agent_rows;
pub use agent_rows::{exists, list_pending_for_agent, pending_agent};

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
