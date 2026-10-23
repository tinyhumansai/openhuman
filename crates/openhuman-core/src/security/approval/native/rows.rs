//! The host-served store behind the module's `ApprovalStore2` and `AuditSink2`
//! callbacks, on the `tinystoragedrivers` document port.
//!
//! The module owns what a row means; this file only keeps its bytes. Every
//! write is a compare-and-swap on the row's version, a mutation ledger returns
//! the original receipt when the module retries an indeterminate write, and the
//! audit ledger accepts exactly the contiguous, identity-checked batches the
//! module sends.
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `approval_rows` | request id | one row's latest version and its queryable fields |
//! | `approval_mutations` | scope digest and mutation id | the content of each accepted write |
//! | `approval_events` | stream and sequence | one audit event |
//! | `approval_tails` | stream | the committed audit tail |
//! | `approval_streams` | subject digest | the subject's persisted audit stream id |
//! | `approval_meta` | request id | what the host chose beside the module's row |

use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinysecurity_bus::{
    ApprovalCasRequest2, ApprovalCasResponse2, ApprovalLifecycle2, ApprovalRecord2, ApprovalRow2,
    AuditEventReceipt2, AuditSinkRequest2, AuditSinkResponse2, SubjectScope,
};
use tinystoragedrivers::{
    CollectionSpec, ErrorKind, Filter, IndexSpec, Precondition, Query, Sort, Versioned,
};

use crate::config::Config;
use crate::storage::{local, DocumentStore, StorageError};

pub(super) const ROWS: &str = "approval_rows";
const MUTATIONS: &str = "approval_mutations";
const EVENTS: &str = "approval_events";
const TAILS: &str = "approval_tails";
const STREAMS: &str = "approval_streams";
const META: &str = "approval_meta";

const DOMAIN: &str = "approval::rows";

/// Rows `Load` returns: the most recently changed. The module rejects a larger
/// answer, and a request it needs is always among the latest ones.
pub(super) const LOAD_LIMIT: usize = 1000;

/// Attempts at advancing a contended tail before giving up.
const TAIL_ATTEMPTS: usize = 8;

/// The collections this store keeps.
pub(super) fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(ROWS)
            .index(IndexSpec::new("by_scope", ["scope", "updated_ts"]))
            .index(IndexSpec::new("by_pending", ["pending", "expires_ts"]))
            .index(IndexSpec::new("by_decided", ["pending", "decided_ts"])),
        CollectionSpec::new(MUTATIONS),
        CollectionSpec::new(EVENTS),
        CollectionSpec::new(TAILS),
        CollectionSpec::new(STREAMS),
        CollectionSpec::new(META),
    ]
}

fn storage(error: StorageError) -> String {
    format!("{DOMAIN}: {error}")
}

/// The sort key of a scope handle: its three digests, joined.
pub(super) fn scope_key(scope: &SubjectScope) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        scope.user, scope.workspace, scope.agent
    )
}

/// A short stable document id for an unbounded string.
pub(super) fn digest_id(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    let full = format!("{:x}", hash.finalize());
    full[..32].to_owned()
}

fn lifecycle_name(state: ApprovalLifecycle2) -> &'static str {
    match state {
        ApprovalLifecycle2::Pending => "pending",
        ApprovalLifecycle2::Approved => "approved",
        ApprovalLifecycle2::Denied => "denied",
        ApprovalLifecycle2::Expired => "expired",
        ApprovalLifecycle2::Cancelled => "cancelled",
        ApprovalLifecycle2::Reserved => "reserved",
        ApprovalLifecycle2::Executed => "executed",
    }
}

fn number(doc: &Value, field: &str) -> Option<i64> {
    doc.get(field).and_then(Value::as_i64)
}

fn text<'a>(doc: &'a Value, field: &str) -> Option<&'a str> {
    doc.get(field).and_then(Value::as_str)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A stored row with the host's own bookkeeping.
#[derive(Clone, Debug)]
pub(super) struct StoredRow {
    /// The module's row.
    pub row: ApprovalRow2,
    /// When the row first left `Pending`, epoch milliseconds.
    pub decided_ms: Option<i64>,
}

/// One scope's document store.
#[derive(Clone)]
pub(super) struct Rows {
    docs: Arc<dyn DocumentStore>,
    ready: Arc<tokio::sync::OnceCell<()>>,
}

impl std::fmt::Debug for Rows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rows").finish_non_exhaustive()
    }
}

impl Rows {
    /// The store of the current call: the storage backend, in the acting scope.
    ///
    /// # Errors
    ///
    /// When the scope cannot be resolved (SaaS mode with no acting profile), so
    /// the call fails instead of reading a shared bucket.
    pub(super) fn current(config: &Config) -> anyhow::Result<Self> {
        let opened = local::open(DOMAIN, &config.workspace_dir)?;
        let scoped = opened.scoped()?;
        Ok(Self::over(Arc::clone(scoped.documents())))
    }

    /// The store over an existing document handle.
    pub(super) fn over(docs: Arc<dyn DocumentStore>) -> Self {
        Self {
            docs,
            ready: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    async fn ready(&self) -> Result<(), String> {
        self.ready
            .get_or_try_init(|| async {
                for spec in collections() {
                    self.docs.ensure_collection(&spec).await?;
                }
                Ok::<(), StorageError>(())
            })
            .await
            .map(|_| ())
            .map_err(storage)
    }

    /// The module's `Load`: the rows most recently changed under `scope`.
    pub(super) async fn load(&self, scope: &SubjectScope) -> Result<Vec<ApprovalRecord2>, String> {
        self.ready().await?;
        let query = Query::filter(Filter::eq("scope", scope_key(scope)))
            .sort(Sort::desc("updated_ts"))
            .limit(LOAD_LIMIT);
        let page = self.docs.query(ROWS, &query).await.map_err(storage)?;
        page.items.iter().map(envelope).collect()
    }

    /// The module's `Cas`: compare-and-swap the row, returning the original
    /// receipt when the same mutation is retried.
    pub(super) async fn cas(
        &self,
        request: &ApprovalCasRequest2,
    ) -> Result<ApprovalCasResponse2, String> {
        self.ready().await?;
        let key = scope_key(&request.scope);
        let ledger_id = digest_id(&[&key, &request.mutation_id]);
        let next_version = request
            .expected_version
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("approval version overflow")?;
        let receipt = || ApprovalCasResponse2 {
            scope: request.scope.clone(),
            record_id: request.record_id.clone(),
            mutation_id: request.mutation_id.clone(),
            resulting_version: next_version,
        };
        let record = &request.next_record;
        if let Some(prior) = self
            .docs
            .get(MUTATIONS, &ledger_id)
            .await
            .map_err(storage)?
        {
            // The same mutation again: only identical content may reuse its identity.
            if text(&prior.doc, "record_json") != Some(record.record_json.as_str())
                || text(&prior.doc, "record_id") != Some(request.record_id.as_str())
            {
                return Err("a mutation identity was reused with other content".into());
            }
            // Already applied: the original receipt, however far the row has moved.
            if prior.doc.get("applied") == Some(&Value::Bool(true)) {
                return Ok(receipt());
            }
            // Recorded but not marked: finish what the interrupted write left.
            let current = self
                .docs
                .get(ROWS, &request.record_id)
                .await
                .map_err(storage)?;
            let landed = current.as_ref().is_some_and(|found| {
                number(&found.doc, "version") == i64::try_from(next_version).ok()
                    && text(&found.doc, "record") == Some(record.record_json.as_str())
            });
            if !landed {
                self.apply(request, next_version, current).await?;
            }
            self.mark_applied(&ledger_id, request, next_version).await;
            return Ok(receipt());
        }
        let current = self
            .docs
            .get(ROWS, &request.record_id)
            .await
            .map_err(storage)?;
        // The intent first: a crash between the two writes is repaired by the retry.
        let intent = json!({
            "record_id": request.record_id,
            "record_json": record.record_json,
            "version": next_version,
            "applied": false,
        });
        match self
            .docs
            .put(MUTATIONS, &ledger_id, intent, Precondition::Absent)
            .await
        {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::Conflict => {}
            Err(error) => return Err(storage(error)),
        }
        self.apply(request, next_version, current).await?;
        self.mark_applied(&ledger_id, request, next_version).await;
        Ok(receipt())
    }

    /// Note that the mutation's row write landed, so a later retry answers from
    /// the ledger alone. A failure here is repaired by that retry.
    async fn mark_applied(&self, ledger_id: &str, request: &ApprovalCasRequest2, version: u64) {
        let applied = json!({
            "record_id": request.record_id,
            "record_json": request.next_record.record_json,
            "version": version,
            "applied": true,
        });
        if let Err(error) = self
            .docs
            .put(MUTATIONS, ledger_id, applied, Precondition::None)
            .await
        {
            tracing::debug!(error = %error, "[approval::rows] the mutation was not marked applied");
        }
    }

    /// Write the row if it is still at the version the mutation expects.
    async fn apply(
        &self,
        request: &ApprovalCasRequest2,
        next_version: u64,
        current: Option<Versioned<Value>>,
    ) -> Result<(), String> {
        let precondition = match (&request.expected_version, &current) {
            (None, None) => Precondition::Absent,
            (Some(expected), Some(found))
                if number(&found.doc, "version") == i64::try_from(*expected).ok() =>
            {
                found.unchanged()
            }
            _ => return Err("the stored row is not at the expected version".into()),
        };
        let row: ApprovalRow2 = serde_json::from_str(&request.next_record.record_json)
            .map_err(|_| "the record does not decode".to_string())?;
        let now = now_ms();
        let pending = row.state == ApprovalLifecycle2::Pending;
        let decided_ts = if pending {
            None
        } else {
            Some(
                current
                    .as_ref()
                    .and_then(|found| number(&found.doc, "decided_ts"))
                    .unwrap_or(now),
            )
        };
        let mut doc = json!({
            "scope": scope_key(&request.scope),
            "version": next_version,
            "state": lifecycle_name(row.state),
            "pending": pending,
            "created_ts": i64::try_from(row.created_at_ms).unwrap_or(i64::MAX),
            "expires_ts": i64::try_from(row.expires_at_ms).unwrap_or(i64::MAX),
            "updated_ts": now,
            "policy_id": request.next_record.policy_id,
            "generation": i64::try_from(request.next_record.generation).unwrap_or(i64::MAX),
            "fingerprint": request.next_record.fingerprint,
            "record": request.next_record.record_json,
        });
        if let Some(decided) = decided_ts {
            doc["decided_ts"] = json!(decided);
        }
        self.docs
            .put(ROWS, &request.record_id, doc, precondition)
            .await
            .map(|_| ())
            .map_err(storage)
    }

    /// One row by request id, whatever its state.
    pub(super) async fn row(&self, request_id: &str) -> Result<Option<StoredRow>, String> {
        self.ready().await?;
        let Some(stored) = self.docs.get(ROWS, request_id).await.map_err(storage)? else {
            return Ok(None);
        };
        Ok(Some(decode_row(&stored)?))
    }

    /// Rows that left `Pending`, newest decision first.
    pub(super) async fn decided(&self, limit: usize) -> Result<Vec<StoredRow>, String> {
        self.ready().await?;
        let query = Query::filter(Filter::eq("pending", false))
            .sort(Sort::desc("decided_ts"))
            .limit(limit);
        let page = self.docs.query(ROWS, &query).await.map_err(storage)?;
        page.items.iter().map(decode_row).collect()
    }

    /// Pending rows whose window elapsed, oldest first.
    pub(super) async fn stale(&self, now: u64, limit: usize) -> Result<Vec<StoredRow>, String> {
        self.ready().await?;
        let cutoff = i64::try_from(now).unwrap_or(i64::MAX);
        let query =
            Query::filter(Filter::eq("pending", true).and(Filter::lte("expires_ts", cutoff)))
                .sort(Sort::asc("expires_ts"))
                .limit(limit);
        let page = self.docs.query(ROWS, &query).await.map_err(storage)?;
        page.items.iter().map(decode_row).collect()
    }

    /// The module's `Append`: accept a contiguous batch, or the replay of one
    /// already committed. Any other overlap is a conflict.
    pub(super) async fn append(
        &self,
        request: &AuditSinkRequest2,
    ) -> Result<AuditSinkResponse2, String> {
        self.ready().await?;
        request
            .validate()
            .map_err(|_| "the audit batch is malformed".to_string())?;
        // A batch may start at the committed tail or replay an earlier one; it may
        // never leave a gap, so nothing is written for one that does.
        if request.expected_tail > self.tail(&request.stream_id).await? {
            return Err("the audit batch leaves a gap after the committed tail".into());
        }
        let mut last = None;
        for event in &request.events {
            let id = format!("{}:{:020}", request.stream_id, event.sequence);
            let doc = json!({
                "event_id": event.event_id,
                "sequence": event.sequence,
                "event": serde_json::to_string(event).map_err(|e| e.to_string())?,
            });
            match self.docs.put(EVENTS, &id, doc, Precondition::Absent).await {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::Conflict => {
                    let stored = self.docs.get(EVENTS, &id).await.map_err(storage)?;
                    if stored.as_ref().and_then(|s| text(&s.doc, "event_id"))
                        != Some(event.event_id.as_str())
                    {
                        return Err("an audit sequence holds another event".into());
                    }
                }
                Err(error) => return Err(storage(error)),
            }
            last = Some(event.sequence);
        }
        let new_tail = last.ok_or("the audit batch is empty")?;
        self.advance_tail(&request.stream_id, request.expected_tail, new_tail)
            .await?;
        Ok(AuditSinkResponse2 {
            scope: request.scope.clone(),
            stream_id: request.stream_id.clone(),
            events: request
                .events
                .iter()
                .map(|event| AuditEventReceipt2 {
                    sequence: event.sequence,
                    event_id: event.event_id.clone(),
                })
                .collect(),
            new_tail,
        })
    }

    async fn advance_tail(&self, stream: &str, expected: u64, new_tail: u64) -> Result<(), String> {
        for _ in 0..TAIL_ATTEMPTS {
            let stored = self.docs.get(TAILS, stream).await.map_err(storage)?;
            let tail = stored
                .as_ref()
                .and_then(|s| number(&s.doc, "tail"))
                .and_then(|n| u64::try_from(n).ok())
                .unwrap_or(0);
            if expected > tail {
                return Err("the audit batch leaves a gap after the committed tail".into());
            }
            if new_tail <= tail {
                return Ok(());
            }
            let precondition = stored
                .as_ref()
                .map_or(Precondition::Absent, Versioned::unchanged);
            let doc = json!({ "tail": i64::try_from(new_tail).map_err(|e| e.to_string())? });
            match self.docs.put(TAILS, stream, doc, precondition).await {
                Ok(_) => return Ok(()),
                Err(error) if error.kind() == ErrorKind::Conflict => {}
                Err(error) => return Err(storage(error)),
            }
        }
        Err("the audit tail kept changing".into())
    }

    /// The committed tail of an audit stream.
    pub(super) async fn tail(&self, stream: &str) -> Result<u64, String> {
        self.ready().await?;
        let stored = self.docs.get(TAILS, stream).await.map_err(storage)?;
        Ok(stored
            .as_ref()
            .and_then(|s| number(&s.doc, "tail"))
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0))
    }

    /// The persisted audit stream of a subject, created on first use.
    pub(super) async fn stream_for(&self, subject: &str) -> Result<String, String> {
        self.ready().await?;
        let id = digest_id(&[subject]);
        for _ in 0..3 {
            if let Some(found) = self.docs.get(STREAMS, &id).await.map_err(storage)? {
                if let Some(stream) = text(&found.doc, "stream_id") {
                    return Ok(stream.to_owned());
                }
            }
            let stream = uuid::Uuid::new_v4().simple().to_string();
            match self
                .docs
                .put(
                    STREAMS,
                    &id,
                    json!({ "stream_id": stream }),
                    Precondition::Absent,
                )
                .await
            {
                Ok(_) => return Ok(stream),
                Err(error) if error.kind() == ErrorKind::Conflict => {}
                Err(error) => return Err(storage(error)),
            }
        }
        Err("the audit stream of the subject could not be created".into())
    }

    /// Remember what the user chose beside the module's row.
    pub(super) async fn put_label(&self, request_id: &str, label: &str) -> Result<(), String> {
        self.ready().await?;
        self.docs
            .put(
                META,
                request_id,
                json!({ "label": label }),
                Precondition::None,
            )
            .await
            .map(|_| ())
            .map_err(storage)
    }

    /// What the user chose beside the module's row.
    pub(super) async fn label(&self, request_id: &str) -> Result<Option<String>, String> {
        self.ready().await?;
        Ok(self
            .docs
            .get(META, request_id)
            .await
            .map_err(storage)?
            .and_then(|found| text(&found.doc, "label").map(str::to_owned)))
    }
}

/// The module's record envelope of a stored row, exactly as it was written.
fn envelope(stored: &Versioned<Value>) -> Result<ApprovalRecord2, String> {
    let field = |name: &str| {
        text(&stored.doc, name)
            .map(str::to_owned)
            .ok_or_else(|| format!("{DOMAIN}: a stored row has no {name}"))
    };
    Ok(ApprovalRecord2 {
        record_id: stored.id.clone(),
        version: number(&stored.doc, "version")
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| format!("{DOMAIN}: a stored row has no version"))?,
        policy_id: field("policy_id")?,
        generation: number(&stored.doc, "generation")
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| format!("{DOMAIN}: a stored row has no generation"))?,
        fingerprint: field("fingerprint")?,
        record_json: field("record")?,
    })
}

fn decode_row(stored: &Versioned<Value>) -> Result<StoredRow, String> {
    let raw = text(&stored.doc, "record").ok_or_else(|| format!("{DOMAIN}: no record"))?;
    let row: ApprovalRow2 =
        serde_json::from_str(raw).map_err(|_| format!("{DOMAIN}: a stored row does not decode"))?;
    Ok(StoredRow {
        row,
        decided_ms: number(&stored.doc, "decided_ts"),
    })
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
