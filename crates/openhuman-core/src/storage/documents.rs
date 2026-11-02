//! A small base for domain stores on the document port.
//!
//! Most core stores have a synchronous API and a fixed set of collections.
//! [`Repo`] holds one scoped document handle, declares the domain's
//! collections before each call, and runs the call on the shared blocking
//! bridge ([`super::block_on`]). [`compare_and_swap`] is the read, change,
//! conditional-write loop every guarded SQL `UPDATE … WHERE` becomes, so two
//! processes on one database never both apply a change that requires a
//! particular prior state, on tsd's `Repo` and `RetryPolicy`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tinystoragedrivers::WriteCondition;
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, Versioned};

use super::{
    block_on, current_scope, installed, DocumentStore, Scope, Storage, StorageBackend, StorageError,
};

/// Collections already declared, per process: `(backend address, scope,
/// collection)`. The `Weak` guards against an address reused by a later
/// backend, so a replaced backend is declared afresh.
type DeclaredKey = (usize, String, String);

static DECLARED: LazyLock<Mutex<HashMap<DeclaredKey, Weak<dyn StorageBackend>>>> =
    LazyLock::new(Mutex::default);

fn backend_addr(backend: &Arc<dyn StorageBackend>) -> usize {
    Arc::as_ptr(backend).cast::<()>() as usize
}

/// Whether `collection` was already declared on `backend` under `scope`.
fn is_declared(backend: &Arc<dyn StorageBackend>, scope: &str, collection: &str) -> bool {
    let key = (
        backend_addr(backend),
        scope.to_string(),
        collection.to_string(),
    );
    DECLARED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .and_then(Weak::upgrade)
        .is_some_and(|alive| Arc::ptr_eq(&alive, backend))
}

/// Records that `collection` is declared on `backend` under `scope`, and
/// forgets entries whose backend is gone.
fn mark_declared(backend: &Arc<dyn StorageBackend>, scope: &str, collection: &str) {
    let key = (
        backend_addr(backend),
        scope.to_string(),
        collection.to_string(),
    );
    let mut declared = DECLARED.lock().unwrap_or_else(PoisonError::into_inner);
    declared.retain(|_, weak| weak.strong_count() > 0);
    declared.insert(key, Arc::downgrade(backend));
}

/// One-time work already done this process (a legacy import): `(backend
/// address, scope, domain, key)`, with the same `Weak` guard as [`DECLARED`].
type OnceKey = (usize, String, &'static str, String);

static DONE_ONCE: LazyLock<Mutex<HashMap<OnceKey, Weak<dyn StorageBackend>>>> =
    LazyLock::new(Mutex::default);

type PreparedKey = (usize, String);
static PREPARED_SCOPES: LazyLock<Mutex<HashMap<PreparedKey, Weak<dyn StorageBackend>>>> =
    LazyLock::new(Mutex::default);

/// Prepares `scope` on `backend` ([`super::boot::prepare_scope`]: owner
/// collections and the graph and harness migrations) once per process, for
/// sync stores that open a scope directly.
///
/// # Errors
///
/// When the backend refuses the scope or the preparation fails; the next
/// call retries.
pub(crate) fn prepare_once(
    backend: &Arc<dyn StorageBackend>,
    scope: &tinystoragedrivers::Scope,
) -> Result<(), StorageError> {
    let prepared_key = (backend_addr(backend), scope.as_str().to_owned());
    let already_prepared = PREPARED_SCOPES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&prepared_key)
        .and_then(Weak::upgrade)
        .is_some_and(|alive| Arc::ptr_eq(&alive, backend));
    if already_prepared {
        return Ok(());
    }
    let to_prepare = backend.for_scope(scope)?;
    let scope = scope.clone();
    block_on(async move {
        super::boot::prepare_scope(&to_prepare, &scope).await?;
        Ok(())
    })?;
    let mut prepared = PREPARED_SCOPES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    prepared.retain(|_, weak| weak.strong_count() > 0);
    prepared.insert(prepared_key, Arc::downgrade(backend));
    Ok(())
}

/// The backend and scope a [`Repo`] was opened on, for declaring its
/// collections once per process instead of on every call.
#[derive(Clone)]
struct Origin {
    backend: Arc<dyn StorageBackend>,
    scope: String,
}

/// One domain's document store under one storage scope.
#[derive(Clone)]
pub struct Repo {
    scoped: Storage,
    docs: Arc<dyn DocumentStore>,
    domain: &'static str,
    collections: fn() -> Vec<CollectionSpec>,
    origin: Option<Origin>,
}

impl Repo {
    /// The repo for this call when the host configured a backend, scoped to
    /// the acting agent; `None` keeps the domain on its classic store.
    ///
    /// # Errors
    ///
    /// When the scope cannot be resolved — in SaaS mode with no acting agent
    /// — so the call fails instead of reading a shared bucket.
    pub fn current(
        domain: &'static str,
        collections: fn() -> Vec<CollectionSpec>,
    ) -> Result<Option<Self>> {
        let Some(backend) = installed() else {
            return Ok(None);
        };
        let scope =
            current_scope().with_context(|| format!("[{domain}] resolve the storage scope"))?;
        Self::on(&backend, &scope, domain, collections).map(Some)
    }

    /// The repo for a store keyed by a workspace: the process's backend (or the
    /// workspace's own default when none is installed,
    /// [`super::local::backend_for`]) under the acting scope.
    ///
    /// # Errors
    ///
    /// When the backend cannot be had or the scope cannot be resolved (SaaS mode
    /// with no acting profile).
    pub fn current_for(
        workspace_dir: &std::path::Path,
        domain: &'static str,
        collections: fn() -> Vec<CollectionSpec>,
    ) -> Result<Self> {
        let opened = super::local::open(domain, workspace_dir)?;
        Self::on(&opened.backend, &opened.scope, domain, collections)
    }

    /// The repo over `backend` under `scope`, declaring its collections once
    /// per process.
    ///
    /// # Errors
    ///
    /// When the backend refuses the scope.
    pub fn on(
        backend: &Arc<dyn StorageBackend>,
        scope: &Scope,
        domain: &'static str,
        collections: fn() -> Vec<CollectionSpec>,
    ) -> Result<Self> {
        let scoped = backend
            .for_scope(scope)
            .with_context(|| format!("[{domain}] open the storage scope"))?;
        prepare_once(backend, scope)
            .with_context(|| format!("[{domain}] prepare owner collections"))?;
        let mut repo = Self::over(&scoped, domain, collections);
        repo.origin = Some(Origin {
            backend: Arc::clone(backend),
            scope: scope.as_str().to_string(),
        });
        Ok(repo)
    }

    /// The repo over an already scoped handle (tests, explicit scopes).
    pub fn over(
        scoped: &Storage,
        domain: &'static str,
        collections: fn() -> Vec<CollectionSpec>,
    ) -> Self {
        Self {
            scoped: scoped.clone(),
            docs: Arc::clone(scoped.documents()),
            domain,
            collections,
            origin: None,
        }
    }

    /// The scoped storage this repo was opened on, for the primitives that
    /// bind to a whole scope (`tsd.migrations`).
    pub fn scoped(&self) -> &Storage {
        &self.scoped
    }

    pub(crate) fn origin_scope(&self) -> Option<&str> {
        self.origin.as_ref().map(|origin| origin.scope.as_str())
    }

    /// Opens the same owner scope with an additional storage-driver write
    /// condition. The backend remains responsible for composing any host
    /// profile fence with `condition` atomically. Collection declarations
    /// are not bound to it (see `claim_bound`).
    ///
    /// Repos made with [`Repo::over`] have no backend/scope origin and cannot
    /// be rebound safely; callers must use an explicitly opened repo for
    /// claim-bound imports.
    pub(crate) fn conditioned(&self, condition: &WriteCondition) -> Result<Self> {
        let origin = self.origin.as_ref().ok_or_else(|| {
            anyhow!(
                "[{}] claim-bound storage requires an opened backend scope",
                self.domain
            )
        })?;
        let scope = Scope::new(&origin.scope)
            .with_context(|| format!("[{}] parse the storage scope", self.domain))?;
        let scoped = super::claim_bound::claim_bound(origin.backend.as_ref(), &scope, condition)
            .with_context(|| {
                format!("[{}] bind the import claim to storage writes", self.domain)
            })?;
        let mut repo = Self::over(&scoped, self.domain, self.collections);
        repo.origin = Some(origin.clone());
        Ok(repo)
    }

    /// Runs `f` once per process for this repo's backend instance, scope and
    /// `key` (a legacy import keyed by its source path). Calls are serialized,
    /// and the run is recorded only when `f` succeeds, so a failed one is
    /// retried by the next call. A replaced backend runs it afresh. A repo
    /// opened with [`Repo::over`] has no backend identity and runs `f` on
    /// every call, so `f` must be idempotent.
    ///
    /// # Errors
    ///
    /// Whatever `f` returns.
    pub fn once_per_backend(&self, key: &str, f: impl FnOnce() -> Result<()>) -> Result<()> {
        let Some(origin) = &self.origin else {
            return f();
        };
        let once_key = (
            backend_addr(&origin.backend),
            origin.scope.clone(),
            self.domain,
            key.to_string(),
        );
        let mut done = DONE_ONCE.lock().unwrap_or_else(PoisonError::into_inner);
        let already = done
            .get(&once_key)
            .and_then(Weak::upgrade)
            .is_some_and(|alive| Arc::ptr_eq(&alive, &origin.backend));
        if already {
            return Ok(());
        }
        f()?;
        done.retain(|_, weak| weak.strong_count() > 0);
        done.insert(once_key, Arc::downgrade(&origin.backend));
        log::debug!("[storage] {} one-time work done", self.domain);
        Ok(())
    }

    /// Runs `op` against the store from synchronous code, after declaring the
    /// domain's collections (idempotent).
    ///
    /// # Errors
    ///
    /// Any storage error, prefixed with the domain name.
    pub fn run<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Arc<dyn DocumentStore>) -> Fut,
        Fut: Future<Output = Result<T, StorageError>> + Send + 'static,
        T: Send + 'static,
    {
        let docs = Arc::clone(&self.docs);
        let origin = self.origin.clone();
        let specs: Vec<CollectionSpec> = (self.collections)()
            .into_iter()
            .filter(|spec| {
                origin
                    .as_ref()
                    .is_none_or(|origin| !is_declared(&origin.backend, &origin.scope, &spec.name))
            })
            .collect();
        let future = op(Arc::clone(&docs));
        let domain = self.domain;
        block_on(async move {
            for spec in &specs {
                docs.ensure_collection(spec).await?;
                if let Some(origin) = &origin {
                    mark_declared(&origin.backend, &origin.scope, &spec.name);
                }
            }
            future.await
        })
        .map_err(|error| anyhow!("[{domain}] storage: {error}"))
    }

    /// Runs an operation with the complete scope-bound six-port handle after
    /// declaring this owner's document collections.
    pub fn run_storage<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: FnOnce(Storage) -> Fut,
        Fut: Future<Output = Result<T, StorageError>> + Send + 'static,
        T: Send + 'static,
    {
        let storage = self.scoped.clone();
        self.run(move |_| op(storage))
    }
}

/// Applies `change` to document `id` in `collection` under compare-and-swap
/// and returns what was stored, or `None` when the document is missing or
/// `change` declines (it is no longer in the state the change requires).
///
/// The operation retries a conditional write up to 32 times when another
/// writer wins the compare-and-swap race.
///
/// # Errors
///
/// A storage error, or a conflict when the document kept changing for the
/// whole retry budget.
pub async fn compare_and_swap(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    change: impl Fn(&Value) -> Option<Value> + Send + Sync,
) -> Result<Option<Versioned<Value>>, StorageError> {
    update_document(docs, collection, id, |current| Ok(change(current))).await
}

/// Applies a fallible transformation to an existing document under
/// compare-and-swap. Returning `Ok(None)` declines the write.
pub async fn update_document(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    mut change: impl FnMut(&Value) -> Result<Option<Value>, StorageError> + Send,
) -> Result<Option<Versioned<Value>>, StorageError> {
    for _ in 0..32 {
        let Some(current) = docs.get(collection, id).await? else {
            return Ok(None);
        };
        let Some(next) = change(&current.doc)? else {
            return Ok(None);
        };
        match docs
            .put(collection, id, next.clone(), current.unchanged())
            .await
        {
            Ok(version) => {
                return Ok(Some(Versioned {
                    id: id.to_owned(),
                    version,
                    doc: next,
                    meta: current.meta,
                }));
            }
            Err(error) if error.kind() == ErrorKind::Conflict => continue,
            Err(error) => return Err(error),
        }
    }
    Err(StorageError::conflict(format!(
        "document `{collection}/{id}` kept changing during compare-and-swap"
    )))
}

/// Repeatedly applies a transformation to the latest document, inserting it
/// when absent. Returning `None` declines the write. Conflicts restart the
/// transformation against a fresh version, so callers never overwrite a
/// concurrent update based on a stale read.
///
/// # Errors
/// A storage error, or a conflict when the document changed for the retry
/// budget.
pub async fn update_or_insert(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    mut change: impl FnMut(Option<&Value>) -> Option<Value> + Send,
) -> Result<bool, StorageError> {
    for _ in 0..32 {
        let current = docs.get(collection, id).await?;
        let Some(next) = change(current.as_ref().map(|record| &record.doc)) else {
            return Ok(false);
        };
        let precondition = current
            .as_ref()
            .map_or(Precondition::Absent, Versioned::unchanged);
        match docs.put(collection, id, next, precondition).await {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == ErrorKind::Conflict => continue,
            Err(error) => return Err(error),
        }
    }
    Err(StorageError::conflict(format!(
        "document `{collection}/{id}` kept changing during update"
    )))
}

/// The string field `field` of `doc`, when present.
pub fn text<'a>(doc: &'a Value, field: &str) -> Option<&'a str> {
    doc.get(field).and_then(Value::as_str)
}

#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;
