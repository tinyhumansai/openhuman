//! Read-only document adapter for one-time import of historical approvals.
//!
//! TinySecurity owns all new approval lifecycle state. This module only reads
//! rows migrated from the retired host store so NativeEngine can import old
//! decisions and flow grants once.

#[cfg(feature = "security-module")]
use std::sync::Arc;

#[cfg(feature = "security-module")]
use anyhow::{anyhow, Result};
#[cfg(feature = "security-module")]
use chrono::{DateTime, Utc};
#[cfg(feature = "security-module")]
use serde_json::Value;
#[cfg(feature = "security-module")]
use tinystoragedrivers::{Filter, Query, Sort, Versioned};

#[cfg(feature = "security-module")]
use crate::config::Config;
#[cfg(feature = "security-module")]
use crate::storage::local;
#[cfg(feature = "security-module")]
use crate::storage::{block_on, DocumentStore, DocumentStoreExt, Storage, StorageError};

#[cfg(feature = "legacy-import")]
use super::storage_schema::legacy_collections;
#[cfg(feature = "security-module")]
pub(super) use super::storage_schema::{APPROVALS, FLOW_TRUST};
#[cfg(feature = "security-module")]
use super::types::{ApprovalAuditEntry, ApprovalDecision};

#[cfg(feature = "security-module")]
const DOMAIN: &str = "approval history import";

#[cfg(feature = "security-module")]
#[derive(Clone, Debug)]
pub(super) struct Docs {
    docs: Arc<dyn DocumentStore>,
}

/// Opens the operator's document scope for the one-time history import.
#[cfg(feature = "security-module")]
pub(super) fn current(config: &Config) -> Result<Docs> {
    let opened = local::open(DOMAIN, &config.workspace_dir)?;
    Ok(Docs::new(&opened.scoped()?))
}

/// The legacy approval database source consumed by the storage importer.
#[cfg(feature = "legacy-import")]
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

#[cfg(feature = "security-module")]
fn from_ms(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::from_timestamp_millis)
}

#[cfg(feature = "security-module")]
fn millis(doc: &Value, field: &str) -> Option<i64> {
    doc.get(field).and_then(Value::as_i64)
}

#[cfg(feature = "security-module")]
fn text<'a>(doc: &'a Value, field: &str) -> Option<&'a str> {
    doc.get(field).and_then(Value::as_str)
}

#[cfg(feature = "security-module")]
fn to_audit(stored: &Versioned<Value>) -> Option<ApprovalAuditEntry> {
    let doc = &stored.doc;
    let decided_at = from_ms(millis(doc, "decided_ms"))?;
    let decision = ApprovalDecision::from_str(text(doc, "decision")?)?;
    let args_redacted = text(doc, "args_redacted")
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(Value::Null);
    Some(ApprovalAuditEntry {
        request_id: stored.id.clone(),
        tool_name: text(doc, "tool_name").unwrap_or_default().to_string(),
        action_summary: text(doc, "action_summary").unwrap_or_default().to_string(),
        args_redacted,
        created_at: from_ms(millis(doc, "created_ms")).unwrap_or_else(Utc::now),
        expires_at: from_ms(millis(doc, "deadline_ms")),
        decided_at,
        decision,
        decided_by: None,
        judge: None,
    })
}

#[cfg(feature = "security-module")]
impl Docs {
    pub(super) fn new(scoped: &Storage) -> Self {
        Self {
            docs: Arc::clone(scoped.documents()),
        }
    }

    fn run<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Arc<dyn DocumentStore>) -> Fut,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'static,
        T: Send + 'static,
    {
        let docs = Arc::clone(&self.docs);
        let future = op(Arc::clone(&docs));
        block_on(async move {
            for spec in super::storage_schema::specs() {
                docs.ensure_collection(&spec).await?;
            }
            future.await
        })
        .map_err(|error| anyhow!("[approval history import] storage: {error}"))
    }

    /// Reads recent historical decisions for TinySecurity's one-time import.
    #[cfg(feature = "security-module")]
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

    /// Reads historical flow grants for TinySecurity's one-time import.
    #[cfg(feature = "security-module")]
    pub(super) fn list_all_flow_trust(&self) -> Result<Vec<(String, String)>> {
        self.run(|docs| async move {
            Ok(docs
                .query_all(FLOW_TRUST, &Query::default())
                .await?
                .iter()
                .filter_map(|grant| {
                    Some((
                        text(&grant.doc, "flow_id")?.to_owned(),
                        text(&grant.doc, "tool_name")?.to_owned(),
                    ))
                })
                .collect())
        })
    }
}
