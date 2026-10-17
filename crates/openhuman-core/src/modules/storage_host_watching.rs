//! A test backend whose documents report a native watch.
//!
//! The in-memory driver has no watch, and the host serves `WatchOpen` from the
//! driver's native watch (it never polls on a module's behalf). This wraps a
//! `MemoryStorage` and fires an `Invalidated` notice at every watcher of a
//! scope when one of its documents is written or deleted, which is all the
//! watch tests need to prove how the signal travels.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tinystoragedrivers::{
    async_trait, Capabilities, Capability, CollectionSpec, DocumentStore, DocumentWatch, Filter,
    MemoryStorage, Page, Precondition, Query, Result, Scope, ScopedStorage, Sort, StorageBackend,
    Version, Versioned, WatchNotice, WatchOptions, WriteOp, WriteResult,
};
use tokio::sync::broadcast;

type Hub = Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>;

/// Memory storage with a native document watch.
#[derive(Debug, Default)]
pub(super) struct WatchingStorage {
    inner: MemoryStorage,
    hub: Hub,
}

impl WatchingStorage {
    pub(super) fn new() -> Self {
        Self::default()
    }
}

impl StorageBackend for WatchingStorage {
    fn driver(&self) -> &'static str {
        "memory"
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities().with(Capability::Watch)
    }

    fn database(&self, name: &str) -> Result<Arc<dyn StorageBackend>> {
        self.inner.database(name)
    }

    fn for_scope(&self, scope: &Scope) -> Result<ScopedStorage> {
        let scoped = self.inner.for_scope(scope)?;
        let sender = {
            let mut hub = self.hub.lock().unwrap();
            hub.entry(scope.as_str().to_string())
                .or_insert_with(|| broadcast::channel(64).0)
                .clone()
        };
        Ok(ScopedStorage::new(
            scope.clone(),
            "memory",
            Arc::new(Documents {
                inner: Arc::clone(scoped.documents()),
                changed: sender,
            }),
            Arc::clone(scoped.streams()),
            Arc::clone(scoped.blobs()),
        ))
    }
}

#[derive(Debug)]
struct Documents {
    inner: Arc<dyn DocumentStore>,
    changed: broadcast::Sender<()>,
}

#[derive(Debug)]
struct Feed(broadcast::Receiver<()>);

#[async_trait]
impl DocumentWatch for Feed {
    async fn next(&mut self) -> Result<Option<WatchNotice>> {
        match self.0.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                Ok(Some(WatchNotice::Invalidated { cursor: None }))
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }
}

#[async_trait]
impl DocumentStore for Documents {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities().with(Capability::Watch)
    }

    async fn watch(
        &self,
        _collection: &str,
        _filter: &Filter,
        _options: &WatchOptions,
    ) -> Result<Box<dyn DocumentWatch>> {
        Ok(Box::new(Feed(self.changed.subscribe())))
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> Result<()> {
        self.inner.ensure_collection(spec).await
    }

    async fn get(&self, c: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        self.inner.get(c, id).await
    }

    async fn put(&self, c: &str, id: &str, doc: Value, p: Precondition) -> Result<Version> {
        let version = self.inner.put(c, id, doc, p).await?;
        let _ = self.changed.send(());
        Ok(version)
    }

    async fn delete(&self, c: &str, id: &str, p: Precondition) -> Result<bool> {
        let deleted = self.inner.delete(c, id, p).await?;
        let _ = self.changed.send(());
        Ok(deleted)
    }

    async fn query(&self, c: &str, q: &Query) -> Result<Page<Versioned<Value>>> {
        self.inner.query(c, q).await
    }

    async fn count(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.count(c, f).await
    }

    async fn delete_where(&self, c: &str, f: &Filter) -> Result<u64> {
        let deleted = self.inner.delete_where(c, f).await?;
        let _ = self.changed.send(());
        Ok(deleted)
    }

    async fn claim(
        &self,
        c: &str,
        f: &Filter,
        s: &[Sort],
        p: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        let claimed = self.inner.claim(c, f, s, p).await?;
        let _ = self.changed.send(());
        Ok(claimed)
    }

    async fn drop_collection(&self, c: &str) -> Result<()> {
        self.inner.drop_collection(c).await
    }

    async fn collections(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.collections(prefix).await
    }

    async fn atomic_batch(&self, ops: Vec<WriteOp>) -> Result<Vec<WriteResult>> {
        let results = self.inner.atomic_batch(ops).await?;
        let _ = self.changed.send(());
        Ok(results)
    }
}
