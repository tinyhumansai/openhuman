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
//! This file is that object and nothing more. It holds no cron policy: every
//! request is delegated mechanically to the scope's [`DocumentStore`] and the
//! reply preserves the error kind so the module's compare-and-swap loops still
//! see `conflict`. The only host rule is the allowlist: the port serves the
//! four schedule collections and refuses any other, so a module bug cannot
//! reach another domain's records.
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
use tinystoragedrivers::{
    Capability, CollectionSpec, DocumentStore, Filter, Precondition, Query, Scope, StorageBackend,
    StorageError, WriteOp,
};

use crate::config::Config;
use crate::modules::tinyflows as module;

/// The collections the schedule store owns.
///
/// The v2 names (`cron.*`, `tsd.*`) are the current module's. The v1 names stay
/// admitted so a tinyflows module release that predates the schema (the pinned
/// artifact is re-pinned only after the epic is released) keeps working against
/// a fresh database; its v1 collections are not migrated by the owner's steps.
const COLLECTIONS: [&str; 10] = [
    "cron.jobs",
    "cron.runs",
    "cron.admission",
    "cron_job_policies",
    "tsd.counters",
    "tsd.leases",
    "tsd.migrations",
    "cron_jobs",
    "cron_runs",
    "cron_counters",
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
            backend: None,
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

    fn documents(&self) -> Result<Arc<dyn DocumentStore>, StorageError> {
        let backend = match &self.backend {
            Some(backend) => Arc::clone(backend),
            None => crate::storage::local::backend_for(&self.workspace_dir)
                .map_err(|error| StorageError::unavailable(error.to_string()))?,
        };
        let scoped = backend.for_scope(&self.scope)?;
        Ok(Arc::clone(scoped.documents()))
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
            "capabilities" => Ok(
                json!({"transactions": documents.capabilities().contains(Capability::Transactions)}),
            ),
            "ensure" => {
                let spec: CollectionSpec = field(&request, "spec")?;
                allowed(&spec.name)?;
                documents.ensure_collection(&spec).await?;
                Ok(Value::Null)
            }
            "get" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                encode(documents.get(collection, text(&request, "id")?).await?)
            }
            "put" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                let precondition: Precondition = field(&request, "precondition")?;
                let doc: Value = field(&request, "doc")?;
                encode(
                    documents
                        .put(collection, text(&request, "id")?, doc, precondition)
                        .await?,
                )
            }
            "delete" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                let precondition: Precondition = field(&request, "precondition")?;
                encode(
                    documents
                        .delete(collection, text(&request, "id")?, precondition)
                        .await?,
                )
            }
            "query" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                let query: Query = field(&request, "query")?;
                encode(documents.query(collection, &query).await?)
            }
            "count" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                let filter: Filter = field(&request, "filter")?;
                encode(documents.count(collection, &filter).await?)
            }
            "delete_where" => {
                let collection = text(&request, "collection")?;
                allowed(collection)?;
                let filter: Filter = field(&request, "filter")?;
                encode(documents.delete_where(collection, &filter).await?)
            }
            "atomic_batch" => {
                let ops: Vec<WriteOp> = field(&request, "ops")?;
                for operation in &ops {
                    match operation {
                        WriteOp::Put { collection, .. } | WriteOp::Delete { collection, .. } => {
                            allowed(collection)?;
                        }
                    }
                }
                encode(documents.atomic_batch(ops).await?)
            }
            _ => Err(StorageError::invalid_input("unexpected document request")),
        }
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
