//! [`FencedBackend`]: a storage backend whose writes pass the lease fence.
//!
//! Wraps any backend. Every scope it hands out is the inner backend's, with
//! the document, stream and blob handles wrapped so that each write first
//! asks the [`FenceRegistry`] for the scope's fence and runs only if that
//! fence passes ([`FenceRegistry::admit`]). Reads, `ensure_collection`
//! (idempotent schema) and scopes with no fence go straight through.
//!
//! A write the host check admits then goes through the inner backend's
//! fenced handles ([`StorageBackend::for_scope_fenced`] with
//! [`LeaseFence::driver_fence`](super::fence::LeaseFence::driver_fence)), so
//! the driver re-checks the lease record atomically with the write. A
//! driver refusal ([`ErrorKind::Fenced`]) latches the fence and surfaces as
//! [`FenceError::Superseded`](super::fence::FenceError::Superseded). Where
//! the driver cannot fence (no [`Capability::Fencing`], a write it answers
//! with `Unsupported(Fencing)`, or a lease store with no
//! record in the ports) the write runs after the host check alone.
//!
//! The fence is looked up per write, not when the scope is bound, because
//! long-lived consumers (the session store caches its per-agent handles)
//! must follow the profile as it is opened, fenced and re-opened.
//!
//! See [`super::fence`] for what the checks do and do not guarantee.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tinystoragedrivers::{
    Blob, BlobMeta, BlobStore, BlobStream, Capabilities, Capability, CollectionSpec, ErrorKind,
    Filter, Page, Precondition, Query, ReadCheck, Result, SearchHit, Sort, StorageError,
    StreamEntry, StreamStore, Version, Versioned, WriteOp, WriteResult,
};

use super::fence::{FenceRegistry, LeaseFence};
use super::{DocumentStore, Scope, ScopedStorage, StorageBackend};

/// A backend whose writes are fenced. See the module docs.
pub struct FencedBackend {
    inner: Arc<dyn StorageBackend>,
    fences: Arc<FenceRegistry>,
}

impl std::fmt::Debug for FencedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FencedBackend")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl FencedBackend {
    /// `inner`, its writes checked against `fences`.
    pub fn new(inner: Arc<dyn StorageBackend>, fences: Arc<FenceRegistry>) -> Self {
        Self { inner, fences }
    }
}

impl FencedBackend {
    /// `scoped`'s handles with every write passing the scope's fence.
    fn wrap_scoped(&self, scope: &Scope, scoped: &ScopedStorage) -> ScopedStorage {
        let guard = Guard {
            scope: scope.clone(),
            fences: Arc::clone(&self.fences),
            backend: Some(Arc::clone(&self.inner)),
        };
        ScopedStorage::new(
            scope.clone(),
            scoped.driver(),
            Arc::new(FencedDocuments {
                inner: Arc::clone(scoped.documents()),
                guard: guard.clone(),
            }),
            Arc::new(FencedStreams {
                inner: Arc::clone(scoped.streams()),
                guard: guard.clone(),
            }),
            Arc::new(FencedBlobs {
                inner: Arc::clone(scoped.blobs()),
                guard,
            }),
        )
    }
}

/// `inner` fenced by the process's registry ([`super::fence::registry`]).
pub fn wrap(inner: Arc<dyn StorageBackend>) -> Arc<dyn StorageBackend> {
    Arc::new(FencedBackend::new(inner, super::fence::registry()))
}

#[async_trait]
impl StorageBackend for FencedBackend {
    fn driver(&self) -> &'static str {
        self.inner.driver()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn for_scope(&self, scope: &Scope) -> Result<ScopedStorage> {
        let scoped = self.inner.for_scope(scope)?;
        Ok(self.wrap_scoped(scope, &scoped))
    }

    fn document_driver(&self) -> &'static str {
        self.inner.document_driver()
    }

    fn existing_schema(&self) -> Result<Arc<dyn StorageBackend>> {
        Ok(Arc::new(Self {
            inner: self.inner.existing_schema()?,
            fences: Arc::clone(&self.fences),
        }))
    }

    async fn prepare_schema(&self) -> Result<()> {
        self.inner.prepare_schema().await
    }

    /// The driver's own fenced handles; the fence wrapper itself builds on
    /// these, so they are not wrapped a second time.
    fn for_scope_fenced(
        &self,
        scope: &Scope,
        fence: &tinystoragedrivers::Fence,
    ) -> Result<ScopedStorage> {
        self.inner.for_scope_fenced(scope, fence)
    }

    fn for_scope_generation(
        &self,
        scope: &Scope,
        generation: tinystoragedrivers::ScopeGeneration,
    ) -> Result<ScopedStorage> {
        let scoped = self.inner.for_scope_generation(scope, generation)?;
        Ok(self.wrap_scoped(scope, &scoped))
    }

    fn for_scope_fenced_generation(
        &self,
        scope: &Scope,
        generation: tinystoragedrivers::ScopeGeneration,
        fence: &tinystoragedrivers::Fence,
    ) -> Result<ScopedStorage> {
        self.inner
            .for_scope_fenced_generation(scope, generation, fence)
    }

    async fn maintain(
        &self,
        budget: tinystoragedrivers::MaintenanceBudget,
    ) -> Result<tinystoragedrivers::MaintenanceReport> {
        self.inner.maintain(budget).await
    }

    async fn checkpoint(&self, mode: tinystoragedrivers::Checkpoint) -> Result<()> {
        self.inner.checkpoint(mode).await
    }

    async fn scopes(&self, prefix: &str) -> Result<Vec<Scope>> {
        self.inner.scopes(prefix).await
    }

    async fn scopes_of_tenant(&self, tenant: &Scope) -> Result<Vec<Scope>> {
        self.inner.scopes_of_tenant(tenant).await
    }

    /// Reclaiming blobs that already read as absent changes nothing a reader
    /// can see, so it needs no fence: it goes straight to the inner driver's
    /// own implementation rather than the trait default over the fenced view.
    async fn sweep_expired_blobs(&self, scope: &Scope) -> Result<u64> {
        self.inner.sweep_expired_blobs(scope).await
    }
}

#[derive(Clone)]
struct Guard {
    scope: Scope,
    fences: Arc<FenceRegistry>,
    /// The inner backend, whose driver may enforce the fence.
    backend: Option<Arc<dyn StorageBackend>>,
}

impl Guard {
    /// The host check. `Ok(Some(..))` when the driver should enforce the
    /// fence too: the fence and the scope's fenced handles to write through.
    async fn admit(&self, op: &'static str) -> Result<Option<(Arc<LeaseFence>, ScopedStorage)>> {
        let Some(lease) = self.fences.admit(self.scope.as_str(), op).await? else {
            return Ok(None);
        };
        let Some(backend) = &self.backend else {
            return Ok(None);
        };
        if !backend.capabilities().contains(Capability::Fencing) {
            return Ok(None);
        }
        let Some(fence) = lease.driver_fence() else {
            return Ok(None);
        };
        match backend.for_scope_fenced(&self.scope, &fence) {
            Ok(scoped) => Ok(Some((lease, scoped))),
            Err(error) if unfenceable(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// What a driver-fenced write's refusal means: a [`ErrorKind::Fenced`]
    /// becomes the latched fence's error; anything else passes through.
    async fn settle<T>(
        &self,
        lease: &LeaseFence,
        op: &'static str,
        outcome: Result<T>,
    ) -> Result<T> {
        match outcome {
            Err(error) if error.kind() == ErrorKind::Fenced => {
                Err(self.fences.driver_refused(lease, op).await)
            }
            other => other,
        }
    }
}

/// The driver cannot make this write atomic with the fence.
fn unfenceable(error: &StorageError) -> bool {
    error.kind() == ErrorKind::Unsupported(Capability::Fencing)
}

/// One fenced write: the host check, then `$port.$method(args)` through the
/// driver's fenced handles when it can enforce the fence, else (or when it
/// answers `Unsupported(Fencing)` for this write) through `$self.inner`.
/// Arguments are cloned for the fenced attempt so the fallback can reuse
/// them.
macro_rules! fenced_write {
    ($self:ident, $op:literal, $port:ident . $method:ident ( $($arg:expr),* $(,)? )) => {{
        match $self.guard.admit($op).await? {
            None => $self.inner.$method($($arg),*).await,
            Some((lease, scoped)) => {
                // `&str` and `Copy` arguments clone trivially; the owned ones
                // must survive for the fallback.
                #[allow(clippy::clone_on_copy)]
                let outcome = scoped.$port().$method($(Clone::clone(&$arg)),*).await;
                match outcome {
                    Err(error) if unfenceable(&error) => {
                        tracing::debug!(
                            target: "openhuman::storage::fence",
                            op = $op,
                            "[storage][fence] driver cannot fence this write; host check only"
                        );
                        $self.inner.$method($($arg),*).await
                    }
                    outcome => $self.guard.settle(&lease, $op, outcome).await,
                }
            }
        }
    }};
}

struct FencedDocuments {
    inner: Arc<dyn DocumentStore>,
    guard: Guard,
}

impl std::fmt::Debug for FencedDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FencedDocuments")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl DocumentStore for FencedDocuments {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> Result<()> {
        self.inner.ensure_collection(spec).await
    }

    async fn get(&self, collection: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        self.inner.get(collection, id).await
    }

    async fn put(
        &self,
        collection: &str,
        id: &str,
        doc: Value,
        precondition: Precondition,
    ) -> Result<Version> {
        fenced_write!(
            self,
            "documents.put",
            documents.put(collection, id, doc, precondition)
        )
    }

    /// Forwarded (and fenced): without it the trait default refuses a write that
    /// declares a schema version or expiry, which is every owner's write.
    async fn put_with(
        &self,
        collection: &str,
        id: &str,
        doc: Value,
        precondition: Precondition,
        options: tinystoragedrivers::WriteOptions,
    ) -> Result<Version> {
        fenced_write!(
            self,
            "documents.put_with",
            documents.put_with(collection, id, doc, precondition, options)
        )
    }

    async fn delete(&self, collection: &str, id: &str, precondition: Precondition) -> Result<bool> {
        fenced_write!(
            self,
            "documents.delete",
            documents.delete(collection, id, precondition)
        )
    }

    async fn query(&self, collection: &str, query: &Query) -> Result<Page<Versioned<Value>>> {
        self.inner.query(collection, query).await
    }

    async fn count(&self, collection: &str, filter: &Filter) -> Result<u64> {
        self.inner.count(collection, filter).await
    }

    async fn delete_where(&self, collection: &str, filter: &Filter) -> Result<u64> {
        fenced_write!(
            self,
            "documents.delete_where",
            documents.delete_where(collection, filter)
        )
    }

    async fn claim(
        &self,
        collection: &str,
        filter: &Filter,
        sort: &[Sort],
        patch: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        fenced_write!(
            self,
            "documents.claim",
            documents.claim(collection, filter, sort, patch)
        )
    }

    async fn atomic_batch(&self, ops: Vec<WriteOp>) -> Result<Vec<WriteResult>> {
        fenced_write!(self, "documents.atomic_batch", documents.atomic_batch(ops))
    }

    async fn atomic_batch_checked(
        &self,
        checks: Vec<ReadCheck>,
        ops: Vec<WriteOp>,
    ) -> Result<Vec<WriteResult>> {
        fenced_write!(
            self,
            "documents.atomic_batch_checked",
            documents.atomic_batch_checked(checks, ops)
        )
    }

    async fn search(&self, collection: &str, text: &str, limit: usize) -> Result<Vec<SearchHit>> {
        self.inner.search(collection, text, limit).await
    }

    async fn drop_collection(&self, collection: &str) -> Result<()> {
        fenced_write!(
            self,
            "documents.drop_collection",
            documents.drop_collection(collection)
        )
    }

    async fn watch(
        &self,
        collection: &str,
        filter: &Filter,
        options: &tinystoragedrivers::WatchOptions,
    ) -> Result<Box<dyn tinystoragedrivers::DocumentWatch>> {
        self.inner.watch(collection, filter, options).await
    }

    async fn validate_collection_spec(&self, required: &CollectionSpec) -> Result<()> {
        self.inner.validate_collection_spec(required).await
    }

    async fn collections(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.collections(prefix).await
    }
}

struct FencedStreams {
    inner: Arc<dyn StreamStore>,
    guard: Guard,
}

impl std::fmt::Debug for FencedStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FencedStreams")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl StreamStore for FencedStreams {
    async fn append(&self, stream: &str, value: Value) -> Result<u64> {
        fenced_write!(self, "streams.append", streams.append(stream, value))
    }

    async fn append_batch(&self, stream: &str, values: Vec<Value>) -> Result<u64> {
        fenced_write!(
            self,
            "streams.append_batch",
            streams.append_batch(stream, values)
        )
    }

    /// Forwarded (and fenced): without it the trait default answers
    /// `Unsupported(ConditionalStreamAppend)` even on a driver that has it,
    /// which refuses every conversation append.
    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> Result<u64> {
        fenced_write!(
            self,
            "streams.append_batch_if_offset",
            streams.append_batch_if_offset(stream, expected_next_offset, values)
        )
    }

    async fn read_window(&self, stream: &str, from: u64, limit: usize) -> Result<Vec<StreamEntry>> {
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(&self, stream: &str, offset: u64) -> Result<u64> {
        fenced_write!(
            self,
            "streams.truncate_before",
            streams.truncate_before(stream, offset)
        )
    }

    async fn delete_stream(&self, stream: &str) -> Result<bool> {
        fenced_write!(self, "streams.delete_stream", streams.delete_stream(stream))
    }

    async fn streams(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.streams(prefix).await
    }

    async fn info(&self, stream: &str) -> Result<Option<tinystoragedrivers::StreamInfo>> {
        self.inner.info(stream).await
    }

    /// Forwarded (and fenced): sealing a finished stream is a write.
    async fn seal(&self, stream: &str) -> Result<tinystoragedrivers::SealOutcome> {
        fenced_write!(self, "streams.seal", streams.seal(stream))
    }

    async fn delete_sealed_stream(&self, stream: &str, sealed_ms: i64) -> Result<bool> {
        fenced_write!(
            self,
            "streams.delete_sealed_stream",
            streams.delete_sealed_stream(stream, sealed_ms)
        )
    }
}

struct FencedBlobs {
    inner: Arc<dyn BlobStore>,
    guard: Guard,
}

impl std::fmt::Debug for FencedBlobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FencedBlobs")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl BlobStore for FencedBlobs {
    fn max_key_len(&self) -> usize {
        self.inner.max_key_len()
    }

    /// The driver's abilities, minus presigned URLs: a presigned PUT is a write
    /// the fence never sees, so the view does not offer them (the trait
    /// default for `presign_get`/`presign_put` is `Unsupported`).
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities().without(Capability::PresignedUrls)
    }

    async fn put_expiring(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: Option<&str>,
        ttl: Duration,
    ) -> Result<BlobMeta> {
        fenced_write!(
            self,
            "blobs.put_expiring",
            blobs.put_expiring(key, bytes, content_type, ttl)
        )
    }

    async fn get_stream(&self, key: &str) -> Result<Option<BlobStream>> {
        self.inner.get_stream(key).await
    }

    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<&str>) -> Result<BlobMeta> {
        fenced_write!(self, "blobs.put", blobs.put(key, bytes, content_type))
    }

    async fn get(&self, key: &str) -> Result<Option<Blob>> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<Vec<u8>>> {
        self.inner.get_range(key, range).await
    }

    async fn head(&self, key: &str) -> Result<Option<BlobMeta>> {
        self.inner.head(key).await
    }

    async fn delete(&self, key: &str) -> Result<bool> {
        fenced_write!(self, "blobs.delete", blobs.delete(key))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>> {
        self.inner.list(prefix).await
    }
}

#[cfg(test)]
#[path = "fenced_backend_tests.rs"]
mod tests;
