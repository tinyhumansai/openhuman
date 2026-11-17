//! The scope-bound document callback for TinyFlows' module-owned catalog.
//!
//! The module owns catalog records and all catalog rules. This host adapter
//! only exposes the acting storage scope through the released CatalogStorage
//! contract. Its callback captures the scope at registration and resolves the
//! workspace's current backend on every request, so backend rebinding is seen
//! without letting a callback select another scope.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use tinybus::Result as BusResult;
use tinyflows_bus::catalog::{
    self, CallbackAddress, CatalogDocumentCursor, CatalogDocumentMeta,
    CatalogStorage as WireStorage, CatalogStorageFailure, CatalogStorageReply,
    CatalogVersionedDocument, DocDeleteRequest, DocDeleteWhereRequest, DocEnsureRequest,
    DocGetRequest, DocPutRequest, DocQueryRequest,
};
use tinystoragedrivers::StorageBackend;
use tinystoragedrivers_api::DocumentStore;
use tinystoragedrivers_types::{
    Capability, CollectionSpec, Filter, Page, Precondition, Query, StorageError, Versioned,
    WriteOptions,
};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::modules::tinyflows as module;
use crate::storage::Scope;

/// A callback scope captured from a dispatch, with no ambient identity or
/// backend handle retained. The workspace backend is resolved for each call.
#[derive(Clone)]
struct DocScope {
    workspace_dir: PathBuf,
    scope: Scope,
    backend: Option<Arc<dyn StorageBackend>>,
}

impl DocScope {
    fn key(&self) -> RegistrationKey {
        RegistrationKey {
            workspace_dir: self.workspace_dir.clone(),
            scope: self.scope.as_str().to_owned(),
        }
    }

    async fn documents(&self) -> Result<Arc<dyn DocumentStore>, StorageError> {
        let backend = match &self.backend {
            Some(backend) => Arc::clone(backend),
            None => crate::storage::local::backend_for_async(&self.workspace_dir)
                .await
                .map_err(|error| StorageError::unavailable(error.to_string()))?,
        };
        let scoped = backend.for_scope(&self.scope)?;
        Ok(Arc::clone(scoped.documents()))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RegistrationKey {
    workspace_dir: PathBuf,
    scope: String,
}

/// One scope's process-lifetime callback registration. TinyFlows uses this
/// address as a run-owner key, so the module runtime owns the registration for
/// the process lifetime along with its scope-only document adapter.
pub(crate) struct CatalogStorageRegistration {
    address: CallbackAddress,
    _served: module::ServedObjectRegistration,
}

impl CatalogStorageRegistration {
    /// The typed callback address to put in Catalog and Runs requests.
    pub(crate) fn storage(&self) -> WireStorage {
        WireStorage {
            callback: self.address.clone(),
        }
    }
}

static REGISTRATIONS: LazyLock<
    tokio::sync::Mutex<HashMap<RegistrationKey, Arc<CatalogStorageRegistration>>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[cfg(test)]
struct RegistrationGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
static REGISTRATION_GATES: LazyLock<std::sync::Mutex<HashMap<String, RegistrationGate>>> =
    LazyLock::new(Default::default);

#[cfg(test)]
static REGISTRATION_SERVE_COUNTS: LazyLock<std::sync::Mutex<HashMap<String, usize>>> =
    LazyLock::new(Default::default);

/// Register or retrieve the CatalogStorage callback for this dispatch's scope.
///
/// Registration work runs on the process-lifetime module runtime. Cancelling
/// the caller after the object is served therefore cannot drop its token before
/// the address is committed to the strong cache.
pub(crate) async fn register(
    context: &DispatchCtx,
    config: &Config,
) -> Result<Arc<CatalogStorageRegistration>, String> {
    let tenant = context.tenant().map_err(|error| error.to_string())?;
    let scope = crate::storage::scope_from(
        tenant.profile.as_deref(),
        tenant.agent.as_deref(),
        crate::core::runtime::is_saas(),
    )
    .map_err(|error| error.to_string())?;
    let documents = DocScope {
        workspace_dir: config.workspace_dir.clone(),
        scope,
        backend: None,
    };
    register_scope(documents).await
}

/// Register a callback for a specific backend and scope, for boot-time owner
/// migrations that must not depend on whichever backend is globally installed.
pub(crate) async fn register_backend_scope(
    config: &Config,
    backend: Arc<dyn StorageBackend>,
    scope: Scope,
) -> Result<Arc<CatalogStorageRegistration>, String> {
    static NEXT_MIGRATION_REGISTRATION: AtomicU64 = AtomicU64::new(1);
    let documents = DocScope {
        workspace_dir: config.workspace_dir.clone(),
        scope,
        backend: Some(backend),
    };
    let nonce = NEXT_MIGRATION_REGISTRATION.fetch_add(1, Ordering::Relaxed);
    let path = format!("{}m{nonce}", path_for(&documents.key()));
    let runtime = crate::modules::runtime::runtime()
        .await
        .map_err(|error| format!("the module bus is unavailable: {error}"))?;
    runtime
        .spawn(async move {
            let served =
                module::serve_registered(path.clone(), CatalogStoragePort { scope: documents })
                    .await?;
            Ok(Arc::new(CatalogStorageRegistration {
                address: CallbackAddress {
                    destination: module::HOST_BUS_NAME.to_owned(),
                    object_path: path,
                },
                _served: served,
            }))
        })
        .await
        .map_err(|error| {
            format!("CatalogStorage migration registration task did not finish: {error}")
        })?
}

async fn register_scope(documents: DocScope) -> Result<Arc<CatalogStorageRegistration>, String> {
    let key = documents.key();
    let path = path_for(&key);
    let runtime = crate::modules::runtime::runtime()
        .await
        .map_err(|error| format!("the module bus is unavailable: {error}"))?;

    runtime
        .spawn(async move {
            let mut registrations = REGISTRATIONS.lock().await;
            if let Some(registration) = registrations.get(&key) {
                return Ok(Arc::clone(registration));
            }

            let served =
                module::serve_registered(path.clone(), CatalogStoragePort { scope: documents })
                    .await?;
            #[cfg(test)]
            record_served(&path);
            #[cfg(test)]
            pause_before_cache_insert(&path).await;
            let registration = Arc::new(CatalogStorageRegistration {
                address: CallbackAddress {
                    destination: module::HOST_BUS_NAME.to_owned(),
                    object_path: path,
                },
                _served: served,
            });
            registrations.insert(key, Arc::clone(&registration));
            Ok(registration)
        })
        .await
        .map_err(|error| format!("CatalogStorage registration task did not finish: {error}"))?
}

/// Register from the existing synchronous flow store API.
pub(crate) fn register_blocking(config: &Config) -> anyhow::Result<WireStorage> {
    let context = DispatchCtx::ambient();
    let config = config.clone();
    crate::storage::block_on_anyhow(async move {
        register(&context, &config)
            .await
            .map(|registration| registration.storage())
            .map_err(anyhow::Error::msg)
    })
}

/// Call the host document callback directly from a scoped unit-test fixture.
#[cfg(test)]
pub(crate) fn call_test_storage<T, A>(
    config: &Config,
    member: &'static str,
    args: A,
) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + Send + 'static,
    A: Serialize + Send + Sync + 'static,
{
    let storage = register_blocking(config)?;
    crate::storage::block_on_anyhow(async move {
        let runtime = crate::modules::runtime::runtime()
            .await
            .map_err(anyhow::Error::msg)?;
        let proxy = runtime
            .connection()
            .proxy(
                &storage.callback.destination,
                &storage.callback.object_path,
                catalog::STORAGE_INTERFACE,
            )
            .map_err(anyhow::Error::msg)?;
        proxy
            .call(member, &args)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    })
}

#[cfg(test)]
async fn pause_before_cache_insert(path: &str) {
    let gate = REGISTRATION_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(path);
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        let _ = gate.release.await;
    }
}

#[cfg(test)]
fn record_served(path: &str) {
    REGISTRATION_SERVE_COUNTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(path.to_owned())
        .and_modify(|count| *count += 1)
        .or_insert(1);
}

fn path_for(key: &RegistrationKey) -> String {
    let workspace = key.workspace_dir.as_os_str().as_encoded_bytes();
    let scope = key.scope.as_bytes();
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"openhuman.tinyflows.catalog-storage.v1\0");
    hasher.update((workspace.len() as u64).to_be_bytes());
    hasher.update(workspace);
    hasher.update((scope.len() as u64).to_be_bytes());
    hasher.update(scope);
    format!(
        "/ai/tinyhumans/openhuman/CatalogStorage/c{}",
        hex::encode(hasher.finalize())
    )
}

/// Catalog-owned collections and their legacy migration sources. Keep this
/// list aligned with the released module adapter; shared TSD collections are
/// further restricted to flow-owned record keys below. The graph checkpointer
/// migration reads its legacy collections through this same scoped port.
const COLLECTIONS: [&str; 25] = [
    "flows.definitions",
    "flows.revisions",
    "flows.runs",
    "flows.suggestions",
    "flows.drafts",
    "graph.checkpoints",
    "graph.threads",
    "graph.writes",
    "graph_checkpoints",
    "graph_threads",
    "graph_writes",
    "delegation_graph_checkpoints",
    "delegation_graph_threads",
    "delegation_graph_writes",
    "flows_graph_checkpoints",
    "flows_graph_threads",
    "flows_graph_writes",
    "flows_definitions",
    "flows_revisions",
    "flows_runs",
    "flows_suggestions",
    "flows_drafts",
    "flows_kv",
    "flows_state",
    "tsd.migrations",
];

const SHARED_COLLECTIONS: [&str; 4] = ["tsd.kv", "tsd.leases", "tsd.counters", "tsd.migrations"];
const GRAPH_COLLECTIONS: [&str; 3] = ["graph.checkpoints", "graph.threads", "graph.writes"];
const FLOW_KEY_PREFIX: &str = "flows/";
const FLOW_MIGRATION_IDS: [&str; 15] = [
    "flows/flows.definitions/definitions_v2",
    "flows/flows.revisions/revisions_v2",
    "flows/flows.runs/runs_v2",
    "flows/flows.suggestions/suggestions_v2",
    "flows/flows.drafts/drafts_v2",
    "flows/kv_import",
    "graph/graph.checkpoints/checkpoints_graph_v2",
    "graph/graph.checkpoints/checkpoints_delegation_v2",
    "graph/graph.checkpoints/checkpoints_flows_v2",
    "graph/graph.threads/threads_graph_v2",
    "graph/graph.threads/threads_delegation_v2",
    "graph/graph.threads/threads_flows_v2",
    "graph/graph.writes/writes_graph_v2",
    "graph/graph.writes/writes_delegation_v2",
    "graph/graph.writes/writes_flows_v2",
];

fn allowed_collection(collection: &str) -> Result<(), StorageError> {
    if COLLECTIONS.contains(&collection) || SHARED_COLLECTIONS.contains(&collection) {
        Ok(())
    } else {
        Err(StorageError::invalid_input(
            "collection is outside the workflow catalog port",
        ))
    }
}

fn allowed_record(collection: &str, id: &str) -> Result<(), StorageError> {
    allowed_collection(collection)?;
    if collection == "tsd.migrations" && !FLOW_MIGRATION_IDS.contains(&id) {
        return Err(StorageError::invalid_input(
            "record is outside the catalog migration progress keys",
        ));
    }
    if GRAPH_COLLECTIONS.contains(&collection)
        && ![FLOW_KEY_PREFIX, "graph/", "delegation/"]
            .iter()
            .any(|prefix| id.starts_with(prefix))
    {
        return Err(StorageError::invalid_input(
            "record is outside the workflow catalog owner's keys",
        ));
    }
    if SHARED_COLLECTIONS.contains(&collection)
        && collection != "tsd.migrations"
        && !id.starts_with(FLOW_KEY_PREFIX)
    {
        return Err(StorageError::invalid_input(
            "record is outside the workflow catalog owner's keys",
        ));
    }
    Ok(())
}

fn allowed_set(collection: &str) -> Result<(), StorageError> {
    allowed_collection(collection)?;
    if SHARED_COLLECTIONS.contains(&collection) {
        Err(StorageError::invalid_input(
            "set operations are outside the workflow catalog port for shared collections",
        ))
    } else {
        Ok(())
    }
}

fn flow_namespace_filter(collection: &str, filter: Filter) -> Filter {
    if GRAPH_COLLECTIONS.contains(&collection) {
        Filter::eq("ns", "flows").and(filter)
    } else {
        filter
    }
}

fn encoded<T: Serialize>(value: T) -> Result<Value, StorageError> {
    serde_json::to_value(value).map_err(|error| {
        StorageError::invalid_input(format!("catalog value encoding failed: {error}"))
    })
}

fn decoded<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, StorageError> {
    serde_json::from_value(value).map_err(|error| {
        StorageError::invalid_input(format!("catalog value decoding failed: {error}"))
    })
}

fn catalog_error(error: StorageError) -> CatalogStorageReply {
    CatalogStorageReply::Error {
        error: CatalogStorageFailure {
            kind: error.kind().to_string().replace(' ', "_"),
            message: error.to_string(),
        },
    }
}

fn catalog_reply<T: Serialize>(result: Result<T, StorageError>) -> CatalogStorageReply {
    match result.and_then(encoded) {
        Ok(value) => CatalogStorageReply::Value { value },
        Err(error) => catalog_error(error),
    }
}

struct CatalogStoragePort {
    scope: DocScope,
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.CatalogStorage")]
impl CatalogStoragePort {
    async fn doc_capabilities(&self) -> BusResult<CatalogStorageReply> {
        let result =
            self.scope
                .documents()
                .await
                .map(|documents| catalog::CatalogStorageCapabilities {
                    record_metadata: documents
                        .capabilities()
                        .contains(Capability::RecordMetadata),
                });
        Ok(catalog_reply(result))
    }

    async fn doc_ensure(&self, request: DocEnsureRequest) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_collection(&request.spec.name)?;
            let spec: CollectionSpec = decoded(encoded(request.spec)?)?;
            self.scope.documents().await?.ensure_collection(&spec).await
        }
        .await;
        Ok(catalog_reply(result))
    }

    async fn doc_get(&self, request: DocGetRequest) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_record(&request.collection, &request.id)?;
            let value: Option<Versioned<Value>> = self
                .scope
                .documents()
                .await?
                .get(&request.collection, &request.id)
                .await?;
            value
                .map(|record| {
                    Ok(CatalogVersionedDocument {
                        id: record.id,
                        version: record.version.0,
                        doc: record.doc,
                        meta: CatalogDocumentMeta {
                            schema: record.meta.schema,
                            created_ms: record.meta.created_ms,
                            updated_ms: record.meta.updated_ms,
                            expires_ms: record.meta.expires_ms,
                        },
                    })
                })
                .transpose()
        }
        .await;
        Ok(catalog_reply(result))
    }

    async fn doc_put(&self, request: DocPutRequest) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_record(&request.collection, &request.id)?;
            let precondition: Precondition = decoded(encoded(request.precondition)?)?;
            let documents = self.scope.documents().await?;
            documents
                .put_with(
                    &request.collection,
                    &request.id,
                    request.doc,
                    precondition,
                    WriteOptions {
                        schema: request.schema,
                        expires_ms: request.expires_ms,
                        created_ms: request.created_ms,
                    },
                )
                .await
        }
        .await;
        Ok(catalog_reply(result))
    }

    async fn doc_delete(&self, request: DocDeleteRequest) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_record(&request.collection, &request.id)?;
            let precondition: Precondition = decoded(encoded(request.precondition)?)?;
            self.scope
                .documents()
                .await?
                .delete(&request.collection, &request.id, precondition)
                .await
        }
        .await;
        Ok(catalog_reply(result))
    }

    async fn doc_query(&self, request: DocQueryRequest) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_set(&request.collection)?;
            let mut query: Query = decoded(encoded(request.query)?)?;
            query.filter = flow_namespace_filter(&request.collection, query.filter);
            let page: Page<Versioned<Value>> = self
                .scope
                .documents()
                .await?
                .query(&request.collection, &query)
                .await?;
            Ok(catalog::CatalogDocumentPage {
                items: page
                    .items
                    .into_iter()
                    .map(|record| CatalogVersionedDocument {
                        id: record.id,
                        version: record.version.0,
                        doc: record.doc,
                        meta: CatalogDocumentMeta {
                            schema: record.meta.schema,
                            created_ms: record.meta.created_ms,
                            updated_ms: record.meta.updated_ms,
                            expires_ms: record.meta.expires_ms,
                        },
                    })
                    .collect(),
                next: page.next.map(|cursor| CatalogDocumentCursor(cursor.0)),
            })
        }
        .await;
        Ok(catalog_reply(result))
    }

    async fn doc_delete_where(
        &self,
        request: DocDeleteWhereRequest,
    ) -> BusResult<CatalogStorageReply> {
        let result = async {
            allowed_set(&request.collection)?;
            let filter: Filter = decoded(encoded(request.filter)?)?;
            let filter = flow_namespace_filter(&request.collection, filter);
            self.scope
                .documents()
                .await?
                .delete_where(&request.collection, &filter)
                .await
        }
        .await;
        Ok(catalog_reply(result))
    }
}

#[cfg(test)]
#[path = "catalog_storage_tests.rs"]
mod tests;

#[cfg(all(test, unix, feature = "storage-sqlite"))]
#[path = "catalog_storage_resource_tests.rs"]
mod resource_tests;
