//! Local lease checks around condition-bound v2 storage handles.
//!
//! The driver condition is the cross-process compare-and-swap boundary. These
//! adapters add the host's stricter local expiry margin and latched-loss
//! behavior for handles that outlive a lease renewal or profile lifecycle.

use std::{ops::Range, sync::Arc, time::Duration};

use async_trait::async_trait;
use serde_json::Value;
use tinystoragedrivers_api::{
    BlobReader, BlobStore, BlobStream, DocumentStore, DocumentWatch, KvStore, QueueStore, Storage,
    StreamStore,
};
use tinystoragedrivers_types::{
    Blob, BlobMeta, Capabilities, Capability, CollectionSpec, Cursor, DeadLetter, Delivery,
    Enqueued, ErrorKind, Filter, NewMessage, Page, Precondition, Query, QueueFeatures, QueueSpec,
    QueueStats, ReadCheck, Receipt, Result, Retry, SealOutcome, SearchHit, Sort, StreamEntry,
    StreamInfo, TreeEntry, TreeFile, Version, Versioned, WatchOptions, WriteOp, WriteOptions,
    WriteResult,
};

use super::{
    fence::{FenceRegistry, LeaseFence},
    StorageError,
};

fn check(fence: &LeaseFence, clock: &FenceRegistry) -> Result<()> {
    fence
        .check_local(clock.now_ms())
        .map_err(|error| error.into_storage())
}

async fn classify_driver_refusal<T>(
    result: Result<T>,
    fence: &LeaseFence,
    clock: &FenceRegistry,
) -> Result<T> {
    match result {
        Err(error) if error.kind() == ErrorKind::Conflict => {
            match fence.check(clock.now_ms()).await {
                Err(fence_error) => Err(fence_error.into_storage()),
                Ok(()) => Err(error),
            }
        }
        other => other,
    }
}

/// `storage` guarded by `fence`. Collection declarations go through
/// `declarations`, a handle without the lease's driver condition: a
/// declaration is idempotent schema metadata, and drivers disagree on
/// whether a conditioned handle may make one (MongoDB refuses, #8422). They
/// still pass the fence's local lease check first.
pub(super) fn wrap(
    storage: Storage,
    declarations: Arc<dyn DocumentStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
) -> Storage {
    Storage::new(
        Arc::new(GuardedDocuments {
            inner: Arc::clone(storage.documents()),
            declarations,
            fence: Arc::clone(&fence),
            clock: Arc::clone(&clock),
        }),
        Arc::new(GuardedStreams {
            inner: Arc::clone(storage.streams()),
            fence: Arc::clone(&fence),
            clock: Arc::clone(&clock),
        }),
        Arc::new(GuardedKv {
            inner: Arc::clone(storage.kv()),
            fence: Arc::clone(&fence),
            clock: Arc::clone(&clock),
        }),
        Arc::new(GuardedBlobs {
            inner: Arc::clone(storage.blobs()),
            fence: Arc::clone(&fence),
            clock: Arc::clone(&clock),
        }),
        Arc::new(GuardedQueues {
            inner: Arc::clone(storage.queues()),
            fence: Arc::clone(&fence),
            clock,
        }),
        Arc::clone(storage.secrets()),
    )
}

struct GuardedDocuments {
    inner: Arc<dyn DocumentStore>,
    /// Collection declarations only, without the lease's driver condition.
    declarations: Arc<dyn DocumentStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
}
impl std::fmt::Debug for GuardedDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedDocuments").finish_non_exhaustive()
    }
}
#[async_trait]
impl DocumentStore for GuardedDocuments {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn watch(&self, c: &str, f: &Filter, o: &WatchOptions) -> Result<Box<dyn DocumentWatch>> {
        self.inner.watch(c, f, o).await
    }
    async fn ensure_collection(&self, s: &CollectionSpec) -> Result<()> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.declarations.ensure_collection(s).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn validate_collection_spec(&self, s: &CollectionSpec) -> Result<()> {
        self.declarations.validate_collection_spec(s).await
    }
    async fn get(&self, c: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        self.inner.get(c, id).await
    }
    async fn put(&self, c: &str, id: &str, v: Value, p: Precondition) -> Result<Version> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.put(c, id, v, p).await, &self.fence, &self.clock).await
    }
    async fn put_with(
        &self,
        c: &str,
        id: &str,
        v: Value,
        p: Precondition,
        o: WriteOptions,
    ) -> Result<Version> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_with(c, id, v, p, o).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn delete(&self, c: &str, id: &str, p: Precondition) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete(c, id, p).await, &self.fence, &self.clock).await
    }
    async fn query(&self, c: &str, q: &Query) -> Result<Page<Versioned<Value>>> {
        self.inner.query(c, q).await
    }
    async fn count(&self, c: &str, f: &Filter) -> Result<u64> {
        self.inner.count(c, f).await
    }
    async fn delete_where(&self, c: &str, f: &Filter) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.delete_where(c, f).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn claim(
        &self,
        c: &str,
        f: &Filter,
        s: &[Sort],
        p: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.claim(c, f, s, p).await, &self.fence, &self.clock).await
    }
    async fn atomic_batch(&self, o: Vec<WriteOp>) -> Result<Vec<WriteResult>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.atomic_batch(o).await, &self.fence, &self.clock).await
    }
    async fn atomic_batch_checked(
        &self,
        c: Vec<ReadCheck>,
        o: Vec<WriteOp>,
    ) -> Result<Vec<WriteResult>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.atomic_batch_checked(c, o).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn search(&self, c: &str, t: &str, l: usize) -> Result<Vec<SearchHit>> {
        self.inner.search(c, t, l).await
    }
    async fn drop_collection(&self, c: &str) -> Result<()> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.drop_collection(c).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn collections(&self, p: &str) -> Result<Vec<String>> {
        self.inner.collections(p).await
    }
}

struct GuardedStreams {
    inner: Arc<dyn StreamStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
}
impl std::fmt::Debug for GuardedStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedStreams").finish_non_exhaustive()
    }
}
#[async_trait]
impl StreamStore for GuardedStreams {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn append(&self, s: &str, v: Value) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.append(s, v).await, &self.fence, &self.clock).await
    }
    async fn append_batch(&self, s: &str, v: Vec<Value>) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.append_batch(s, v).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn append_batch_if_offset(&self, s: &str, o: u64, v: Vec<Value>) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.append_batch_if_offset(s, o, v).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn read_window(&self, s: &str, f: u64, l: usize) -> Result<Vec<StreamEntry>> {
        self.inner.read_window(s, f, l).await
    }
    async fn len(&self, s: &str) -> Result<u64> {
        self.inner.len(s).await
    }
    async fn truncate_before(&self, s: &str, o: u64) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.truncate_before(s, o).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn delete_stream(&self, s: &str) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete_stream(s).await, &self.fence, &self.clock).await
    }
    async fn streams(&self, p: &str) -> Result<Vec<String>> {
        self.inner.streams(p).await
    }
    async fn seal(&self, s: &str) -> Result<SealOutcome> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.seal(s).await, &self.fence, &self.clock).await
    }
    async fn delete_sealed_stream(&self, s: &str, t: i64) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.delete_sealed_stream(s, t).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn info(&self, s: &str) -> Result<Option<StreamInfo>> {
        self.inner.info(s).await
    }
}

struct GuardedKv {
    inner: Arc<dyn KvStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
}
impl std::fmt::Debug for GuardedKv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedKv").finish_non_exhaustive()
    }
}
#[async_trait]
impl KvStore for GuardedKv {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn get(&self, k: &str) -> Result<Option<Versioned<Value>>> {
        self.inner.get(k).await
    }
    async fn put(&self, k: &str, v: Value) -> Result<Version> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.put(k, v).await, &self.fence, &self.clock).await
    }
    async fn cas(&self, k: &str, v: Value, e: Precondition) -> Result<Version> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.cas(k, v, e).await, &self.fence, &self.clock).await
    }
    async fn delete(&self, k: &str, e: Precondition) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete(k, e).await, &self.fence, &self.clock).await
    }
    async fn list(&self, p: &str, c: Option<Cursor>, l: usize) -> Result<Page<Versioned<Value>>> {
        self.inner.list(p, c, l).await
    }
    async fn put_with_ttl(&self, k: &str, v: Value, t: Duration) -> Result<Version> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_with_ttl(k, v, t).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
}

struct GuardedBlobs {
    inner: Arc<dyn BlobStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
}
impl std::fmt::Debug for GuardedBlobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedBlobs").finish_non_exhaustive()
    }
}
#[async_trait]
impl BlobStore for GuardedBlobs {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn max_key_len(&self) -> usize {
        self.inner.max_key_len()
    }
    fn supports_sealed_blobs(&self) -> bool {
        self.inner.supports_sealed_blobs()
    }
    async fn put(&self, k: &str, b: Vec<u8>, c: Option<&str>) -> Result<BlobMeta> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.put(k, b, c).await, &self.fence, &self.clock).await
    }
    async fn put_stream(
        &self,
        k: &str,
        r: BlobReader,
        n: u64,
        c: Option<&str>,
    ) -> Result<BlobMeta> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_stream(k, r, n, c).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn put_expiring(
        &self,
        k: &str,
        b: Vec<u8>,
        c: Option<&str>,
        t: Duration,
    ) -> Result<BlobMeta> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_expiring(k, b, c, t).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn put_stream_expiring(
        &self,
        k: &str,
        r: BlobReader,
        n: u64,
        c: Option<&str>,
        t: Duration,
    ) -> Result<BlobMeta> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_stream_expiring(k, r, n, c, t).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn get(&self, k: &str) -> Result<Option<Blob>> {
        self.inner.get(k).await
    }
    async fn get_stream(&self, k: &str) -> Result<Option<BlobStream>> {
        self.inner.get_stream(k).await
    }
    async fn get_range(&self, k: &str, r: Range<u64>) -> Result<Option<Vec<u8>>> {
        self.inner.get_range(k, r).await
    }
    async fn head(&self, k: &str) -> Result<Option<BlobMeta>> {
        self.inner.head(k).await
    }
    async fn delete(&self, k: &str) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete(k).await, &self.fence, &self.clock).await
    }
    async fn list(&self, p: &str) -> Result<Vec<BlobMeta>> {
        self.inner.list(p).await
    }
    async fn put_sealed(&self, k: &str, b: Vec<u8>, c: Option<&str>) -> Result<BlobMeta> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.put_sealed(k, b, c).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn get_sealed(&self, k: &str) -> Result<Option<Blob>> {
        self.inner.get_sealed(k).await
    }
    async fn head_sealed(&self, k: &str) -> Result<Option<BlobMeta>> {
        self.inner.head_sealed(k).await
    }
    async fn delete_sealed(&self, k: &str) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete_sealed(k).await, &self.fence, &self.clock).await
    }
    async fn list_sealed(&self, p: &str) -> Result<Vec<BlobMeta>> {
        self.inner.list_sealed(p).await
    }
    async fn list_page(
        &self,
        p: &str,
        a: Option<&str>,
        l: usize,
    ) -> Result<tinystoragedrivers_types::BlobListPage> {
        self.inner.list_page(p, a, l).await
    }
    async fn sweep_expired(&self) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.sweep_expired().await, &self.fence, &self.clock).await
    }
    async fn presign_get(&self, k: &str, t: Duration) -> Result<String> {
        self.inner.presign_get(k, t).await
    }
    async fn presign_put(&self, k: &str, t: Duration) -> Result<String> {
        let _ = (k, t);
        Err(StorageError::unsupported(
            Capability::PresignedUrls,
            "condition-bound storage cannot issue a write URL that outlives its lease check",
        ))
    }
    async fn list_dir(&self, d: &str) -> Result<Vec<TreeEntry>> {
        self.inner.list_dir(d).await
    }
    async fn put_tree(&self, d: &str, f: Vec<TreeFile>) -> Result<Vec<BlobMeta>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.put_tree(d, f).await, &self.fence, &self.clock).await
    }
    async fn delete_tree(&self, d: &str) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.delete_tree(d).await, &self.fence, &self.clock).await
    }
}

#[cfg(test)]
#[path = "fenced_blob_stream_tests.rs"]
mod blob_stream_tests;

struct GuardedQueues {
    inner: Arc<dyn QueueStore>,
    fence: Arc<LeaseFence>,
    clock: Arc<FenceRegistry>,
}
impl std::fmt::Debug for GuardedQueues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedQueues").finish_non_exhaustive()
    }
}
#[async_trait]
impl QueueStore for GuardedQueues {
    fn features(&self) -> QueueFeatures {
        self.inner.features()
    }
    async fn ensure_queue(&self, q: &str, s: &QueueSpec) -> Result<()> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.ensure_queue(q, s).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn enqueue(&self, q: &str, m: &[NewMessage]) -> Result<Vec<Enqueued>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.enqueue(q, m).await, &self.fence, &self.clock).await
    }
    async fn receive(&self, q: &str, n: u32, v: u64, c: &str) -> Result<Vec<Delivery>> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(
            self.inner.receive(q, n, v, c).await,
            &self.fence,
            &self.clock,
        )
        .await
    }
    async fn ack(&self, q: &str, r: &Receipt) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.ack(q, r).await, &self.fence, &self.clock).await
    }
    async fn nack(&self, q: &str, r: &Receipt, retry: Retry) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.nack(q, r, retry).await, &self.fence, &self.clock).await
    }
    async fn extend(&self, q: &str, r: &Receipt, v: u64) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.extend(q, r, v).await, &self.fence, &self.clock).await
    }
    async fn stats(&self, q: &str) -> Result<QueueStats> {
        self.inner.stats(q).await
    }
    async fn dead_letters(&self, q: &str, l: usize) -> Result<Vec<DeadLetter>> {
        self.inner.dead_letters(q, l).await
    }
    async fn redrive(&self, q: &str, id: &str) -> Result<bool> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.redrive(q, id).await, &self.fence, &self.clock).await
    }
    async fn purge(&self, q: &str) -> Result<u64> {
        check(&self.fence, &self.clock)?;
        classify_driver_refusal(self.inner.purge(q).await, &self.fence, &self.clock).await
    }
}
