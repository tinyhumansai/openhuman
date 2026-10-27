//! Collection declarations for condition-bound document handles.
//!
//! A driver write condition (a profile lease, an import claim) guards owner
//! data. The MongoDB driver refuses `ensure_collection` on such a handle
//! outright (`Unsupported(ConditionalWrites)`), because a declaration cannot
//! join the condition's transaction. Declarations are idempotent schema
//! merges that carry no owner data, so the host sends them through the same
//! scope's unconditioned handle and keeps the condition on every other call.
//! The fence's local check still runs first (`fenced_ports`), so a node that
//! has lost its lease does not declare either.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinystoragedrivers_api::{DocumentStore, DocumentWatch, Storage};
use tinystoragedrivers_types::{
    Capabilities, CollectionSpec, Filter, Page, Precondition, Query, ReadCheck, Result, SearchHit,
    Sort, Version, Versioned, WatchOptions, WriteOp, WriteOptions, WriteResult,
};

/// `storage` with its collection declarations sent to `declarer`, the
/// unconditioned document handle of the same scope.
pub(super) fn wrap(storage: Storage, declarer: Arc<dyn DocumentStore>) -> Storage {
    Storage::new(
        Arc::new(DeclaringDocuments {
            inner: Arc::clone(storage.documents()),
            declarer,
        }),
        Arc::clone(storage.streams()),
        Arc::clone(storage.kv()),
        Arc::clone(storage.blobs()),
        Arc::clone(storage.queues()),
        Arc::clone(storage.secrets()),
    )
}

struct DeclaringDocuments {
    inner: Arc<dyn DocumentStore>,
    declarer: Arc<dyn DocumentStore>,
}
impl std::fmt::Debug for DeclaringDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclaringDocuments").finish_non_exhaustive()
    }
}
#[async_trait]
impl DocumentStore for DeclaringDocuments {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn watch(&self, c: &str, f: &Filter, o: &WatchOptions) -> Result<Box<dyn DocumentWatch>> {
        self.inner.watch(c, f, o).await
    }
    async fn ensure_collection(&self, s: &CollectionSpec) -> Result<()> {
        log::trace!("[storage] declaring {} outside the write condition", s.name);
        self.declarer.ensure_collection(s).await
    }
    async fn validate_collection_spec(&self, s: &CollectionSpec) -> Result<()> {
        self.inner.validate_collection_spec(s).await
    }
    async fn get(&self, c: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        self.inner.get(c, id).await
    }
    async fn put(&self, c: &str, id: &str, v: Value, p: Precondition) -> Result<Version> {
        self.inner.put(c, id, v, p).await
    }
    async fn put_with(
        &self,
        c: &str,
        id: &str,
        v: Value,
        p: Precondition,
        o: WriteOptions,
    ) -> Result<Version> {
        self.inner.put_with(c, id, v, p, o).await
    }
    async fn delete(&self, c: &str, id: &str, p: Precondition) -> Result<bool> {
        self.inner.delete(c, id, p).await
    }
    async fn query(&self, c: &str, q: &Query) -> Result<Page<Versioned<Value>>> {
        self.inner.query(c, q).await
    }
    async fn count(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.count(c, f).await
    }
    async fn delete_where(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.delete_where(c, f).await
    }
    async fn claim(
        &self,
        c: &str,
        f: &Filter,
        s: &[Sort],
        p: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        self.inner.claim(c, f, s, p).await
    }
    async fn atomic_batch(&self, o: Vec<WriteOp>) -> Result<Vec<WriteResult>> {
        self.inner.atomic_batch(o).await
    }
    async fn atomic_batch_checked(
        &self,
        c: Vec<ReadCheck>,
        o: Vec<WriteOp>,
    ) -> Result<Vec<WriteResult>> {
        self.inner.atomic_batch_checked(c, o).await
    }
    async fn search(&self, c: &str, t: &str, l: usize) -> Result<Vec<SearchHit>> {
        self.inner.search(c, t, l).await
    }
    async fn drop_collection(&self, c: &str) -> Result<()> {
        self.inner.drop_collection(c).await
    }
    async fn collections(&self, p: &str) -> Result<Vec<String>> {
        self.inner.collections(p).await
    }
}
