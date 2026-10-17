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
//! | `approvals.requests` | `request_id` | one approval request and its audit trail |
//! | `approvals.flow_trust` | `Key::of([flow_id, tool_name])` | one "approve always for this flow" grant |
//!
//! The SQL guards become preconditions: a request is inserted only if absent,
//! and deciding, expiring and recording execution are compare-and-swap on the
//! document's version, each applied only while the row is still in the state
//! the SQL `WHERE` clause required (`decided_at IS NULL`, and so on). Two
//! processes sharing one database therefore decide a request at most once. The
//! shapes and the v1 -> v2 steps are in [`super::storage_schema`].
//! `args_redacted` and `source_context` are stored as JSON strings, so a
//! MongoDB-backed port never meets an arbitrary key.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::config::Config;
use crate::storage::documents::compare_and_swap;
use crate::storage::local;
use crate::storage::{block_on, DocumentStore, DocumentStoreExt, ScopedStorage, StorageError};
use tinystoragedrivers::{
    CollectionSpec, ErrorKind, Filter, Key, Precondition, Query, Sort, Versioned,
};

use super::storage_schema::{legacy_collections, specs};
pub(super) use super::storage_schema::{trust_id, APPROVALS, FLOW_TRUST};

use super::types::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, PendingApproval,
};

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
        owner: "approvals",
        db: "approval/approval.db",
        collections: legacy_collections,
        read_legacy: |scratch, _real| super::store_import::read(scratch),
        rekey: |record, _config| record,
    }
}

/// The collections the approval store keeps.
pub(super) fn collections() -> Vec<CollectionSpec> {
    specs().into()
}

fn storage(error: StorageError) -> anyhow::Error {
    anyhow!("[approval::store] storage: {error}")
}

fn from_ms(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::from_timestamp_millis)
}

fn millis(doc: &Value, field: &str) -> Option<i64> {
    doc.get(field).and_then(Value::as_i64)
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
        created_at: from_ms(millis(doc, "created_ms")).unwrap_or_else(Utc::now),
        expires_at: from_ms(millis(doc, "deadline_ms")),
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
    let decided_at = from_ms(millis(&stored.doc, "decided_ms"))?;
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
            "created_ms": pending.created_at.timestamp_millis(),
            "deadline_ms": pending.expires_at.map(|at| at.timestamp_millis()),
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
        let now = Utc::now().timestamp_millis();
        let doc = json!({
            "tool_name": tool_name,
            "action_summary": action_summary,
            "args_redacted": "{}",
            "session_id": session_id,
            "created_ms": now,
            "source_context": serde_json::to_string(source_context)
                .context("[approval::store] serialize preauthorization source_context")?,
            "pending": false,
            "decided_ms": now,
            "decision": decision.as_str(),
        });
        let id = Key::ulid().as_str().to_owned();
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
        let decided_ms = now.timestamp_millis();
        let now_ts = decided_ms;
        self.run(|docs| async move {
            let query =
                Query::filter(Filter::eq("pending", true).and(Filter::lte("deadline_ms", now_ts)));
            let mut expired = Vec::new();
            for stale in docs.query_all(APPROVALS, &query).await? {
                let moved = update(&docs, &stale.id, |doc| {
                    if doc.get("pending") != Some(&json!(true)) {
                        return None;
                    }
                    let mut next = doc.clone();
                    next["pending"] = json!(false);
                    next["decided_ms"] = json!(decided_ms);
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
            let query = Query::filter(Filter::eq("pending", true)).sort(Sort::asc("created_ms"));
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
                millis(&stored.doc, "decided_ms")?;
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
        let decided_ms = at.timestamp_millis();
        self.run(|docs| async move {
            let decided = update(&docs, &id, |doc| {
                if doc.get("pending") != Some(&json!(true)) {
                    return None;
                }
                let mut next = doc.clone();
                next["pending"] = json!(false);
                next["decided_ms"] = json!(decided_ms);
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
        let executed_ms = Utc::now().timestamp_millis();
        let error = audit_error(error);
        self.run(|docs| async move {
            let recorded = update(&docs, &id, |doc| {
                // Decided first, and the first outcome wins.
                if millis(doc, "decided_ms").is_none() || millis(doc, "executed_ms").is_some() {
                    return None;
                }
                let mut next = doc.clone();
                next["executed_ms"] = json!(executed_ms);
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
                    .sort(Sort::desc("decided_ms"))
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
            "created_ms": Utc::now().timestamp_millis(),
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
    change: impl Fn(&Value) -> Option<Value> + Send + Sync,
) -> Result<Option<Versioned<Value>>, StorageError> {
    compare_and_swap(docs, APPROVALS, id, change).await
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
