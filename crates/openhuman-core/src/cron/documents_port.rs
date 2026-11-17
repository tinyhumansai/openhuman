//! The document port the `tinyflows` module persists cron jobs through.
//!
//! The module owns the cron job and run store (`CronDocuments`: claims, CAS
//! loops, history pruning, migrations) but no storage backend. It reaches the
//! scope's documents through one host callback object per scope,
//! `tinyflows_bus::schedule::DOCUMENT_INTERFACE`, whose single member,
//! `Document`, takes a JSON request (`capabilities`, `ensure`, `get`, `put`,
//! `delete`, `query`, `count`, `delete_where`, `atomic_batch`) and answers
//! `{"value": ..}` or `{"error": {"kind", "message"}}`.
//!
//! This file is that object and nothing more. It holds no cron schema or
//! migration logic: every request is delegated mechanically to the scope's
//! [`DocumentStore`] and the reply preserves the error kind so the module's
//! compare-and-swap loops still see `conflict`. Its narrow collection and
//! shared-key checks prevent the module from reaching another domain's
//! records.
//!
//! One object is served per `(workspace, storage scope)`, lazily, and lives for
//! the process. The backend is resolved per request, not captured, so a data
//! reset or a rebound workspace is seen on the next call.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tinybus::Result as BusResult;
use tinyflows_bus::schedule::CallbackAddress;
use tinystoragedrivers::StorageBackend;
use tinystoragedrivers_api::DocumentStore;
use tinystoragedrivers_types::{
    Capability, CollectionSpec, Filter, Precondition, Query, StorageError, WriteOp,
};

use crate::config::Config;
use crate::modules::tinyflows as module;
use crate::storage::Scope;

/// Schedule-owned collections, including the v1 sources read and retired by
/// the module's v1-to-v2 migration.
const COLLECTIONS: [&str; 11] = [
    "cron.jobs",
    "cron.runs",
    "cron.admission",
    "cron_job_policies",
    "tsd.counters",
    "tsd.leases",
    "tsd.migrations",
    "tsd.kv",
    "cron_jobs",
    "cron_runs",
    "cron_counters",
];

/// Shared TSD collections whose records must remain owner-qualified.
const SHARED_COLLECTIONS: [&str; 3] = ["tsd.counters", "tsd.leases", "tsd.migrations"];
/// Prefix emitted by `Key::of(["cron", ..])` for cron-owned records.
const CRON_RECORD_PREFIX: &str = "cron/";
/// Exact TSD migration progress ids owned by the cron schema.
const CRON_MIGRATION_IDS: [&str; 4] = [
    "cron/cron.jobs/jobs_v2",
    "cron/cron.runs/runs_v2",
    "cron/cron.admission/admission_v2",
    "cron/counters_import",
];

/// Where one scope's cron documents live: the workspace whose default backend
/// is used when none is installed, and the storage scope within it.
#[derive(Clone)]
pub(crate) struct DocScope {
    workspace_dir: PathBuf,
    scope: Scope,
    /// A backend to use instead of resolving the workspace's (the legacy
    /// importer runs against the backend it was handed).
    backend: Option<Arc<dyn StorageBackend>>,
}

impl DocScope {
    /// The acting scope (`storage::current_scope()`) of `config`'s workspace.
    ///
    /// # Errors
    ///
    /// SaaS mode with no acting profile, or a backend that cannot be opened.
    pub(crate) fn current(config: &Config) -> Result<Self> {
        let opened = crate::storage::local::open("cron::store", &config.workspace_dir)?;
        Ok(Self {
            workspace_dir: config.workspace_dir.clone(),
            scope: opened.scope,
            // The schedule call crosses the synchronous storage bridge before
            // TinyFlows calls this host document port. Keep the backend we
            // already opened so the callback does not re-enter that bridge.
            backend: Some(opened.backend),
        })
    }

    /// An explicit scope, for a caller that is not running under it (the
    /// hosted-profile sweep reads a closed profile's due jobs).
    pub(crate) fn for_scope(config: &Config, scope: Scope) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
            scope,
            backend: None,
        }
    }

    /// `scope` of an explicit `backend`.
    pub(crate) fn on_backend(
        config: &Config,
        backend: Arc<dyn StorageBackend>,
        scope: Scope,
    ) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
            scope,
            backend: Some(backend),
        }
    }

    fn key(&self) -> String {
        let backend = self
            .backend
            .as_ref()
            .map_or(0, |backend| Arc::as_ptr(backend).cast::<()>() as usize);
        format!(
            "{}|{}|{backend:x}",
            self.workspace_dir.display(),
            self.scope.as_str()
        )
    }

    /// Stable identity for the module's process-local scheduler slot.
    pub(crate) fn scheduler_scope(&self) -> String {
        format!("openhuman-cron:{}", self.key())
    }

    fn documents(&self) -> Result<Arc<dyn DocumentStore>, StorageError> {
        let backend = match &self.backend {
            Some(backend) => Arc::clone(backend),
            None => crate::storage::local::backend_for(&self.workspace_dir)
                .map_err(|error| StorageError::unavailable(error.to_string()))?,
        };
        let scoped = backend.for_scope(&self.scope)?;
        Ok(Arc::clone(scoped.documents()))
    }

    /// Whether this scope can atomically update the schedule store's admission
    /// policy. An unenforced host can skip clearing an absent policy when the
    /// backend lacks transactions; the module refuses writes if a policy
    /// ledger already exists on that backend.
    pub(crate) fn supports_transactions(&self) -> Result<bool, StorageError> {
        Ok(self
            .documents()?
            .capabilities()
            .contains(Capability::Transactions))
    }
}

/// The object. Its interface name is the contract's `DOCUMENT_INTERFACE`
/// (`documents_port_tests` pins them together: the macro takes a literal).
pub(crate) struct DocumentsPort {
    scope: DocScope,
}

fn allowed(collection: &str) -> Result<(), StorageError> {
    if COLLECTIONS.contains(&collection) {
        return Ok(());
    }
    Err(StorageError::invalid_input(
        "collection is outside the schedule port",
    ))
}

fn allowed_record(collection: &str, id: &str) -> Result<(), StorageError> {
    allowed(collection)?;
    if collection == "tsd.kv" {
        return Err(StorageError::invalid_input(
            "the schedule port may only ensure tsd.kv for primitive preparation",
        ));
    }
    if collection == "tsd.migrations" && !CRON_MIGRATION_IDS.contains(&id) {
        return Err(StorageError::invalid_input(
            "record is outside the schedule migration progress keys",
        ));
    }
    if SHARED_COLLECTIONS.contains(&collection) && !id.starts_with(CRON_RECORD_PREFIX) {
        return Err(StorageError::invalid_input(
            "record is outside the schedule owner's keys",
        ));
    }
    Ok(())
}

fn allowed_set(collection: &str) -> Result<(), StorageError> {
    allowed(collection)?;
    if SHARED_COLLECTIONS.contains(&collection) || collection == "tsd.kv" {
        return Err(StorageError::invalid_input(
            "set operations are outside the schedule port for shared collections",
        ));
    }
    Ok(())
}

fn field<T: serde::de::DeserializeOwned>(request: &Value, name: &str) -> Result<T, StorageError> {
    serde_json::from_value(request.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|error| StorageError::invalid_input(format!("bad `{name}`: {error}")))
}

fn text<'a>(request: &'a Value, name: &str) -> Result<&'a str, StorageError> {
    request
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::invalid_input(format!("missing `{name}`")))
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value, StorageError> {
    serde_json::to_value(value).map_err(|error| StorageError::invalid_input(error.to_string()))
}

impl DocumentsPort {
    async fn handle(&self, request: Value) -> Result<Value, StorageError> {
        let documents = self.scope.documents()?;
        let op = text(&request, "op")?.to_string();
        tracing::trace!(%op, "[cron:documents] request");
        match op.as_str() {
            "capabilities" => {
                let capabilities = documents.capabilities();
                Ok(json!({
                    "transactions": capabilities.contains(Capability::Transactions),
                    "record_metadata": capabilities.contains(Capability::RecordMetadata),
                }))
            }
            "ensure" => {
                let spec: CollectionSpec = field(&request, "spec")?;
                allowed(&spec.name)?;
                documents.ensure_collection(&spec).await?;
                Ok(Value::Null)
            }
            "get" => {
                let collection = text(&request, "collection")?;
                let id = text(&request, "id")?;
                allowed_record(collection, id)?;
                encode(documents.get(collection, id).await?)
            }
            "put" => {
                let collection = text(&request, "collection")?;
                let id = text(&request, "id")?;
                allowed_record(collection, id)?;
                let precondition: Precondition = field(&request, "precondition")?;
                let doc: Value = field(&request, "doc")?;
                let options: tinystoragedrivers::WriteOptions =
                    serde_json::from_value(request.clone()).map_err(|error| {
                        StorageError::invalid_input(format!("bad write options: {error}"))
                    })?;
                encode(
                    documents
                        .put_with(collection, id, doc, precondition, options)
                        .await?,
                )
            }
            "delete" => {
                let collection = text(&request, "collection")?;
                let id = text(&request, "id")?;
                allowed_record(collection, id)?;
                let precondition: Precondition = field(&request, "precondition")?;
                encode(documents.delete(collection, id, precondition).await?)
            }
            "query" => {
                let collection = text(&request, "collection")?;
                allowed_set(collection)?;
                let query: Query = field(&request, "query")?;
                encode(documents.query(collection, &query).await?)
            }
            "count" => {
                let collection = text(&request, "collection")?;
                allowed_set(collection)?;
                let filter: Filter = field(&request, "filter")?;
                encode(documents.count(collection, &filter).await?)
            }
            "delete_where" => {
                let collection = text(&request, "collection")?;
                allowed_set(collection)?;
                let filter: Filter = field(&request, "filter")?;
                encode(documents.delete_where(collection, &filter).await?)
            }
            "atomic_batch" => {
                let ops: Vec<WriteOp> = field(&request, "ops")?;
                for operation in &ops {
                    match operation {
                        WriteOp::Put { collection, id, .. }
                        | WriteOp::Delete { collection, id, .. } => {
                            allowed_record(collection, id)?;
                        }
                    }
                }
                #[cfg(test)]
                pause_manual_claim_if_armed(&ops).await;
                encode(documents.atomic_batch(ops).await?)
            }
            _ => Err(StorageError::invalid_input("unexpected document request")),
        }
    }
}

#[cfg(test)]
struct ManualClaimTestGate {
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

#[cfg(test)]
static MANUAL_CLAIM_TEST_GATES: LazyLock<
    std::sync::Mutex<HashMap<String, Arc<ManualClaimTestGate>>>,
> = LazyLock::new(Default::default);

#[cfg(test)]
pub(crate) fn pause_manual_claim_for_test(
    job_id: &str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    MANUAL_CLAIM_TEST_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            job_id.to_string(),
            Arc::new(ManualClaimTestGate {
                entered: std::sync::Mutex::new(Some(entered_tx)),
                release: tokio::sync::Mutex::new(Some(release_rx)),
            }),
        );
    (entered_rx, release_tx)
}

#[cfg(test)]
async fn pause_manual_claim_if_armed(ops: &[WriteOp]) {
    let job_id = ops.iter().find_map(|operation| match operation {
        WriteOp::Put { collection, id, .. }
            if collection == "cron.jobs"
                && MANUAL_CLAIM_TEST_GATES
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains_key(id) =>
        {
            Some(id.clone())
        }
        _ => None,
    });
    let Some(job_id) = job_id else {
        return;
    };
    let gate = MANUAL_CLAIM_TEST_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&job_id);
    let Some(gate) = gate else {
        return;
    };
    if let Some(entered) = gate
        .entered
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        let _ = entered.send(());
    }
    let release = gate.release.lock().await.take();
    if let Some(release) = release {
        let _ = release.await;
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.ScheduleDocuments")]
impl DocumentsPort {
    /// One mechanical document request; see the module docs for the shapes.
    async fn document(&self, request: Value) -> BusResult<Value> {
        Ok(match self.handle(request).await {
            Ok(value) => json!({ "value": value }),
            Err(error) => {
                tracing::debug!(kind = %error.kind(), "[cron:documents] request failed");
                json!({"error": {
                    "kind": error.kind().to_string().replace(' ', "_"),
                    "message": error.to_string(),
                }})
            }
        })
    }
}

/// Addresses already served, by scope key.
static SERVED: LazyLock<tokio::sync::Mutex<HashMap<String, CallbackAddress>>> =
    LazyLock::new(Default::default);

fn path_for(key: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!(
        "/ai/tinyhumans/openhuman/ScheduleDocuments/s{:016x}",
        hasher.finish()
    )
}

/// The address the module calls for `scope`'s documents, serving the object on
/// first use.
///
/// # Errors
///
/// When the module bus is unavailable.
pub(crate) async fn address(scope: &DocScope) -> Result<CallbackAddress> {
    let key = scope.key();
    let mut served = SERVED.lock().await;
    if let Some(address) = served.get(&key) {
        return Ok(address.clone());
    }
    let path = path_for(&key);
    module::serve(
        path.clone(),
        DocumentsPort {
            scope: scope.clone(),
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!(error))
    .context("serve the schedule document port")?;
    tracing::debug!(scope = %scope.scope.as_str(), %path, "[cron:documents] port served");
    let address = CallbackAddress {
        destination: module::HOST_BUS_NAME.to_string(),
        object_path: path,
    };
    served.insert(key, address.clone());
    Ok(address)
}

#[cfg(all(test, feature = "modules"))]
#[path = "documents_port_tests.rs"]
mod tests;
