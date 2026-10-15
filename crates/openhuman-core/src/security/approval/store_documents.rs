//! The approval store on the `tinystoragedrivers` document port.
//!
//! Used instead of `approval/approval.db` when the host configured a storage
//! backend ([`crate::storage`]); the public functions in `store.rs` pick it
//! per call. Records live under the current call's storage scope (the acting
//! agent, `local` on a single-user host), so in a multi-tenant process one
//! user's approvals are invisible to another's.
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `approvals` | `request_id` | one approval request and its audit trail |
//! | `approval_flow_trust` | `(flow_id, tool_name)` | one "approve always for this flow" grant |
//!
//! The SQL guards become preconditions: a request is inserted only if absent,
//! and deciding, expiring and recording execution are compare-and-swap on the
//! document's version, each applied only while the row is still in the state
//! the SQL `WHERE` clause required (`decided_at IS NULL`, and so on). Two
//! processes sharing one database therefore decide a request at most once.
//! `args_redacted` and `source_context` are stored as JSON strings, so a
//! MongoDB-backed port never meets an arbitrary key.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::config::Config;
use crate::storage::local;
use crate::storage::{block_on, DocumentStore, DocumentStoreExt, ScopedStorage, StorageError};
use tinystoragedrivers::{
    CollectionSpec, ErrorKind, Filter, IndexSpec, Precondition, Query, Sort, Versioned,
};

use super::types::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, PendingApproval,
};

pub(super) const APPROVALS: &str = "approvals";
pub(super) const FLOW_TRUST: &str = "approval_flow_trust";

/// Compare-and-swap attempts before a contended update gives up.
const CAS_ATTEMPTS: usize = 32;

const DOMAIN: &str = "approval::store";

/// The approval store over one scoped document handle.
#[derive(Clone, Debug)]
pub(super) struct Docs {
    docs: Arc<dyn DocumentStore>,
}

/// The document store for this call: the storage backend, in the acting scope.
///
/// # Errors
///
/// When the storage scope cannot be resolved — in SaaS mode with no acting
/// agent — so the call fails rather than reading a shared bucket.
pub(super) fn current(config: &Config) -> Result<Docs> {
    let opened = local::open(DOMAIN, &config.workspace_dir)?;
    Ok(Docs::new(&opened.scoped()?))
}

/// The store of embedded agent `agent`'s scope, whichever scope the caller
/// runs in.
pub(super) fn for_agent(config: &Config, agent: &str) -> Result<Docs> {
    let opened = local::open(DOMAIN, &config.workspace_dir)?;
    let scoped = opened
        .backend
        .for_scope(&crate::storage::scope_for_agent(agent))
        .map_err(|error| anyhow!("[approval::store] open the agent's scope: {error}"))?;
    Ok(Docs::new(&scoped))
}

/// The legacy `approval.db`, as the storage importer reads it.
pub(crate) fn import_source() -> crate::storage::import::Source {
    crate::storage::import::Source {
        domain: "approval",
        db: "approval/approval.db",
        collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

/// The collections the approval store keeps.
pub(super) fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(APPROVALS)
            .index(IndexSpec::new("by_pending", ["pending", "created_at"]))
            .index(IndexSpec::new("by_decided", ["pending", "decided_at"]))
            .index(IndexSpec::new("by_session", ["session_id", "pending"])),
        CollectionSpec::new(FLOW_TRUST).index(IndexSpec::new("by_flow", ["flow_id", "tool_name"])),
    ]
}

fn storage(error: StorageError) -> anyhow::Error {
    anyhow!("[approval::store] storage: {error}")
}

/// Fixed-width (nanosecond, `Z`) so the strings sort in time order.
pub(super) fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// The comparable expiry key: nanoseconds since the epoch, falling back to
/// milliseconds for instants beyond the i64-nanosecond range (year 2262).
pub(super) fn expiry_key(at: DateTime<Utc>) -> i64 {
    at.timestamp_nanos_opt()
        .unwrap_or_else(|| at.timestamp_millis().saturating_mul(1_000_000))
}

fn parse_time(raw: Option<&str>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|at| at.with_timezone(&Utc))
}

fn text<'a>(doc: &'a Value, field: &str) -> Option<&'a str> {
    doc.get(field).and_then(Value::as_str)
}

/// A stored request as a [`PendingApproval`].
fn to_pending(stored: &Versioned<Value>) -> PendingApproval {
    let doc = &stored.doc;
    PendingApproval {
        request_id: stored.id.clone(),
        tool_name: text(doc, "tool_name").unwrap_or_default().to_string(),
        action_summary: text(doc, "action_summary").unwrap_or_default().to_string(),
        args_redacted: text(doc, "args_redacted")
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or(Value::Null),
        created_at: parse_time(text(doc, "created_at")).unwrap_or_else(Utc::now),
        expires_at: parse_time(text(doc, "expires_at")),
        source_context: text(doc, "source_context").and_then(|raw| {
            serde_json::from_str::<ApprovalSourceContext>(raw)
                .map_err(|error| {
                    tracing::warn!(
                        %error,
                        "[approval::store] failed to decode source_context JSON — treating as absent"
                    );
                })
                .ok()
        }),
        tool_call_id: text(doc, "tool_call_id").map(str::to_string),
        agent_id: text(doc, "agent_id").map(str::to_string),
    }
}

/// A decided request as an [`ApprovalAuditEntry`]; `None` if it is not
/// decided or its decision is unknown.
fn to_audit(stored: &Versioned<Value>) -> Option<ApprovalAuditEntry> {
    let pending = to_pending(stored);
    let decided_at = parse_time(text(&stored.doc, "decided_at"))?;
    let decision = ApprovalDecision::from_str(text(&stored.doc, "decision")?)?;
    Some(ApprovalAuditEntry {
        request_id: pending.request_id,
        tool_name: pending.tool_name,
        action_summary: pending.action_summary,
        args_redacted: pending.args_redacted,
        created_at: pending.created_at,
        expires_at: pending.expires_at,
        decided_at,
        decision,
        decided_by: text(&stored.doc, "decided_by").map(str::to_owned),
        judge: stored
            .doc
            .get("judge")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
    })
}

/// `error` sanitized for the durable audit row, then capped at 512 chars —
/// sanitize first, so truncation can never split a secret past the redactor.
fn audit_error(error: Option<&str>) -> Option<String> {
    error.map(|raw| {
        let sanitized = crate::security::scrub::sanitize_text(raw).value;
        if sanitized.chars().count() > 512 {
            let head: String = sanitized.chars().take(511).collect();
            format!("{head}…")
        } else {
            sanitized
        }
    })
}

impl Docs {
    pub(super) fn new(scoped: &ScopedStorage) -> Self {
        Self {
            docs: Arc::clone(scoped.documents()),
        }
    }

    /// Runs `op` against the store from synchronous code.
    fn run<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Arc<dyn DocumentStore>) -> Fut,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'static,
        T: Send + 'static,
    {
        let docs = Arc::clone(&self.docs);
        let future = op(Arc::clone(&docs));
        block_on(async move {
            declare(&docs).await?;
            future.await
        })
        .map_err(storage)
    }

    pub(super) fn insert_pending(&self, pending: &PendingApproval, session_id: &str) -> Result<()> {
        let doc = json!({
            "tool_name": pending.tool_name,
            "action_summary": pending.action_summary,
            "args_redacted": serde_json::to_string(&pending.args_redacted)
                .context("[approval::store] serialize args_redacted")?,
            "session_id": session_id,
            "created_at": rfc3339(pending.created_at),
            "expires_at": pending.expires_at.map(rfc3339),
            "expires_ts": pending.expires_at.map(expiry_key),
            "source_context": pending
                .source_context
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("[approval::store] serialize source_context")?,
            "tool_call_id": pending.tool_call_id,
            "agent_id": pending.agent_id,
            "pending": true,
        });
        let id = pending.request_id.clone();
        self.run(|docs| async move {
            docs.put(APPROVALS, &id, doc, Precondition::Absent)
                .await
                .map(|_| ())
        })
    }

    /// Inserts a request that is decided from birth (a flow
    /// pre-authorization): it never appears pending, only in the audit
    /// trail.
    pub(super) fn insert_decided(
        &self,
        tool_name: &str,
        action_summary: &str,
        session_id: &str,
        source_context: &ApprovalSourceContext,
        decision: ApprovalDecision,
    ) -> Result<()> {
        let now = rfc3339(Utc::now());
        let doc = json!({
            "tool_name": tool_name,
            "action_summary": action_summary,
            "args_redacted": "{}",
            "session_id": session_id,
            "created_at": now,
            "source_context": serde_json::to_string(source_context)
                .context("[approval::store] serialize preauthorization source_context")?,
            "pending": false,
            "decided_at": now,
            "decision": decision.as_str(),
        });
        let id = uuid::Uuid::new_v4().to_string();
        self.run(|docs| async move {
            docs.put(APPROVALS, &id, doc, Precondition::Absent)
                .await
                .map(|_| ())
        })
    }

    /// Moves every undecided request past its expiry to a terminal `deny`
    /// and returns the ones this call moved, so the caller can announce
    /// them.
    pub(super) fn expire_stale(&self, now: DateTime<Utc>) -> Result<Vec<PendingApproval>> {
        let deny = ApprovalDecision::Deny.as_str();
        let decided_at = rfc3339(now);
        let now_ts = expiry_key(now);
        self.run(|docs| async move {
            let query =
                Query::filter(Filter::eq("pending", true).and(Filter::lte("expires_ts", now_ts)));
            let mut expired = Vec::new();
            for stale in docs.query_all(APPROVALS, &query).await? {
                let moved = update(&docs, &stale.id, |doc| {
                    if doc.get("pending") != Some(&json!(true)) {
                        return None;
                    }
                    let mut next = doc.clone();
                    next["pending"] = json!(false);
                    next["decided_at"] = json!(decided_at);
                    next["decision"] = json!(deny);
                    Some(next)
                })
                .await?;
                if let Some(moved) = moved {
                    expired.push(to_pending(&moved));
                }
            }
            Ok(expired)
        })
    }

    pub(super) fn list_pending(&self) -> Result<Vec<PendingApproval>> {
        self.run(|docs| async move {
            let query = Query::filter(Filter::eq("pending", true)).sort(Sort::asc("created_at"));
            Ok(docs
                .query_all(APPROVALS, &query)
                .await?
                .iter()
                .map(to_pending)
                .collect())
        })
    }

    /// The agent that parked the still-undecided `request_id`: `None` when no
    /// such request is pending, `Some(None)` for one the process parked.
    pub(super) fn pending_agent(&self, request_id: &str) -> Result<Option<Option<String>>> {
        let id = request_id.to_string();
        self.run(|docs| async move {
            Ok(docs.get(APPROVALS, &id).await?.and_then(|stored| {
                (stored.doc.get("pending") == Some(&json!(true)))
                    .then(|| text(&stored.doc, "agent_id").map(str::to_string))
            }))
        })
    }

    pub(super) fn get_decision(&self, request_id: &str) -> Result<Option<ApprovalDecision>> {
        let id = request_id.to_string();
        self.run(|docs| async move {
            Ok(docs.get(APPROVALS, &id).await?.and_then(|stored| {
                text(&stored.doc, "decided_at")?;
                ApprovalDecision::from_str(text(&stored.doc, "decision")?)
            }))
        })
    }

    pub(super) fn decide(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<Option<PendingApproval>> {
        self.decide_at(request_id, decision, Utc::now())
    }

    /// [`decide`](Self::decide) stamped `at`, so ordering can be pinned.
    pub(super) fn decide_at(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
        at: DateTime<Utc>,
    ) -> Result<Option<PendingApproval>> {
        let id = request_id.to_string();
        let decided_at = rfc3339(at);
        self.run(|docs| async move {
            let decided = update(&docs, &id, |doc| {
                if doc.get("pending") != Some(&json!(true)) {
                    return None;
                }
                let mut next = doc.clone();
                next["pending"] = json!(false);
                next["decided_at"] = json!(decided_at);
                next["decision"] = json!(decision.as_str());
                Some(next)
            })
            .await?;
            Ok(decided.as_ref().map(to_pending))
        })
    }

    pub(super) fn record_execution(
        &self,
        request_id: &str,
        outcome: ExecutionOutcome,
        error: Option<&str>,
    ) -> Result<bool> {
        let id = request_id.to_string();
        let executed_at = rfc3339(Utc::now());
        let error = audit_error(error);
        self.run(|docs| async move {
            let recorded = update(&docs, &id, |doc| {
                // Decided first, and the first outcome wins.
                if text(doc, "decided_at").is_none() || text(doc, "executed_at").is_some() {
                    return None;
                }
                let mut next = doc.clone();
                next["executed_at"] = json!(executed_at);
                next["execution_outcome"] = json!(outcome.as_str());
                next["execution_error"] = json!(error);
                Some(next)
            })
            .await?;
            Ok(recorded.is_some())
        })
    }

    pub(super) fn list_recent_decisions(&self, limit: usize) -> Result<Vec<ApprovalAuditEntry>> {
        self.run(|docs| async move {
            let query =
                Query::filter(Filter::eq("pending", false).and(Filter::exists("decision", true)))
                    .sort(Sort::desc("decided_at"))
                    .limit(limit);
            let page = docs.query(APPROVALS, &query).await?;
            Ok(page.items.iter().filter_map(to_audit).collect())
        })
    }

    pub(super) fn purge_session(&self, session_id: &str) -> Result<usize> {
        let session = session_id.to_string();
        self.run(|docs| async move {
            let removed = docs
                .delete_where(
                    APPROVALS,
                    &Filter::eq("session_id", session).and(Filter::eq("pending", true)),
                )
                .await?;
            Ok(usize::try_from(removed).unwrap_or(usize::MAX))
        })
    }

    pub(super) fn insert_flow_trust(&self, flow_id: &str, tool_name: &str) -> Result<()> {
        let id = trust_id(flow_id, tool_name);
        let doc = json!({
            "flow_id": flow_id,
            "tool_name": tool_name,
            "created_at": rfc3339(Utc::now()),
        });
        self.run(|docs| async move {
            // `INSERT OR IGNORE`: re-granting is a no-op.
            match docs.put(FLOW_TRUST, &id, doc, Precondition::Absent).await {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == ErrorKind::Conflict => Ok(()),
                Err(error) => Err(error),
            }
        })
    }

    pub(super) fn list_flow_trust(&self, flow_id: &str) -> Result<Vec<String>> {
        let flow = flow_id.to_string();
        self.run(|docs| async move {
            let query = Query::filter(Filter::eq("flow_id", flow)).sort(Sort::asc("tool_name"));
            Ok(docs
                .query_all(FLOW_TRUST, &query)
                .await?
                .iter()
                .filter_map(|grant| text(&grant.doc, "tool_name").map(str::to_string))
                .collect())
        })
    }

    pub(super) fn delete_flow_trust(
        &self,
        flow_id: &str,
        tool_names: Option<&[String]>,
    ) -> Result<usize> {
        let flow = flow_id.to_string();
        let names = tool_names.map(<[String]>::to_vec);
        self.run(|docs| async move {
            let removed = match names {
                None => {
                    docs.delete_where(FLOW_TRUST, &Filter::eq("flow_id", flow))
                        .await?
                }
                Some(names) => {
                    let mut removed = 0;
                    for name in names {
                        if docs
                            .delete(FLOW_TRUST, &trust_id(&flow, &name), Precondition::None)
                            .await?
                        {
                            removed += 1;
                        }
                    }
                    removed
                }
            };
            Ok(usize::try_from(removed).unwrap_or(usize::MAX))
        })
    }

    pub(super) fn is_flow_tool_trusted(&self, flow_id: &str, tool_name: &str) -> Result<bool> {
        let id = trust_id(flow_id, tool_name);
        self.run(|docs| async move { Ok(docs.get(FLOW_TRUST, &id).await?.is_some()) })
    }
}

/// The id of a `(flow_id, tool_name)` grant: length-prefixed, so no two
/// pairs collide.
pub(super) fn trust_id(flow_id: &str, tool_name: &str) -> String {
    format!("{}:{flow_id}/{tool_name}", flow_id.len())
}

async fn declare(docs: &Arc<dyn DocumentStore>) -> Result<(), StorageError> {
    for spec in collections() {
        docs.ensure_collection(&spec).await?;
    }
    Ok(())
}

/// Applies `change` to the request `id` under compare-and-swap and returns
/// the stored result, or `None` when the request is missing or `change`
/// declines (the request is no longer in the state the change requires).
async fn update(
    docs: &Arc<dyn DocumentStore>,
    id: &str,
    change: impl Fn(&Value) -> Option<Value>,
) -> Result<Option<Versioned<Value>>, StorageError> {
    for _ in 0..CAS_ATTEMPTS {
        let Some(stored) = docs.get(APPROVALS, id).await? else {
            return Ok(None);
        };
        let Some(next) = change(&stored.doc) else {
            return Ok(None);
        };
        match docs
            .put(APPROVALS, id, next.clone(), stored.unchanged())
            .await
        {
            Ok(version) => {
                return Ok(Some(Versioned {
                    id: id.to_string(),
                    version,
                    doc: next,
                }));
            }
            Err(error) if error.kind() == ErrorKind::Conflict => {}
            Err(error) => return Err(error),
        }
    }
    Err(StorageError::conflict(format!(
        "approval {id} kept changing under {CAS_ATTEMPTS} attempts"
    )))
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
