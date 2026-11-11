//! Storage bound to an import claim that can still declare its collections.
//!
//! An importer writes inside a claim ([`super::import_state::with_claim`]),
//! through storage whose writes carry the claim's `WriteCondition`, so a
//! process that lost its claim cannot write records. Declaring a collection
//! is not such a write: it is an idempotent merge of schema metadata, and a
//! stale process declaring one harms nothing.
//!
//! Drivers disagree on whether a conditioned handle may declare a
//! collection. The memory, file and SQLite drivers allow it; MongoDB refuses
//! with `Unsupported(ConditionalWrites)` ("conditioned MongoDB handles cannot
//! mutate collection declarations"). On MongoDB every claimed import that
//! declared a collection therefore failed, at every boot (#8422).
//!
//! [`claim_bound`] opens the conditioned storage with one change: its
//! document handle declares and validates collections through the scope's
//! unconditioned handle. Every other document operation, and every other
//! port, stays under the claim.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinystoragedrivers::{Scope, WriteCondition};
use tinystoragedrivers_api::{DocumentStore, DocumentWatch, Storage};
use tinystoragedrivers_types::{
    Capabilities, CollectionSpec, Filter, Page, Precondition, Query, ReadCheck, Result, SearchHit,
    Sort, Version, Versioned, WatchOptions, WriteOp, WriteOptions, WriteResult,
};

use super::StorageBackend;

/// `backend`'s storage for `scope` with every write bound to `condition`,
/// except collection declarations, which go through the scope's plain
/// handle.
///
/// # Errors
///
/// The error from opening either the conditioned or the plain handle, such
/// as `Unsupported(ConditionalWrites)` from a backend without conditioned
/// writes.
pub(crate) fn claim_bound(
    backend: &dyn StorageBackend,
    scope: &Scope,
    condition: &WriteCondition,
) -> Result<Storage> {
    let claimed = backend.for_scope_conditioned(scope, condition)?;
    let plain = backend.for_scope(scope)?;
    Ok(Storage::new(
        Arc::new(ClaimBoundDocuments {
            claimed: Arc::clone(claimed.documents()),
            plain: Arc::clone(plain.documents()),
        }),
        Arc::clone(claimed.streams()),
        Arc::clone(claimed.kv()),
        Arc::clone(claimed.blobs()),
        Arc::clone(claimed.queues()),
        Arc::clone(claimed.secrets()),
    ))
}

/// A claim-bound document handle that declares collections unconditioned.
struct ClaimBoundDocuments {
    /// Every read and write, under the claim.
    claimed: Arc<dyn DocumentStore>,
    /// Collection declarations and their validation only.
    plain: Arc<dyn DocumentStore>,
}

impl std::fmt::Debug for ClaimBoundDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaimBoundDocuments")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl DocumentStore for ClaimBoundDocuments {
    fn capabilities(&self) -> Capabilities {
        self.claimed.capabilities()
    }
    async fn watch(&self, c: &str, f: &Filter, o: &WatchOptions) -> Result<Box<dyn DocumentWatch>> {
        self.claimed.watch(c, f, o).await
    }
    async fn ensure_collection(&self, s: &CollectionSpec) -> Result<()> {
        self.plain.ensure_collection(s).await
    }
    async fn validate_collection_spec(&self, s: &CollectionSpec) -> Result<()> {
        self.plain.validate_collection_spec(s).await
    }
    async fn get(&self, c: &str, id: &str) -> Result<Option<Versioned<Value>>> {
        self.claimed.get(c, id).await
    }
    async fn put(&self, c: &str, id: &str, v: Value, p: Precondition) -> Result<Version> {
        self.claimed.put(c, id, v, p).await
    }
    async fn put_with(
        &self,
        c: &str,
        id: &str,
        v: Value,
        p: Precondition,
        o: WriteOptions,
    ) -> Result<Version> {
        self.claimed.put_with(c, id, v, p, o).await
    }
    async fn delete(&self, c: &str, id: &str, p: Precondition) -> Result<bool> {
        self.claimed.delete(c, id, p).await
    }
    async fn query(&self, c: &str, q: &Query) -> Result<Page<Versioned<Value>>> {
        self.claimed.query(c, q).await
    }
    async fn count(&self, c: &str, f: &Filter) -> Result<u64> {
        self.claimed.count(c, f).await
    }
    async fn delete_where(&self, c: &str, f: &Filter) -> Result<u64> {
        self.claimed.delete_where(c, f).await
    }
    async fn claim(
        &self,
        c: &str,
        f: &Filter,
        s: &[Sort],
        p: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        self.claimed.claim(c, f, s, p).await
    }
    async fn atomic_batch(&self, o: Vec<WriteOp>) -> Result<Vec<WriteResult>> {
        self.claimed.atomic_batch(o).await
    }
    async fn atomic_batch_checked(
        &self,
        c: Vec<ReadCheck>,
        o: Vec<WriteOp>,
    ) -> Result<Vec<WriteResult>> {
        self.claimed.atomic_batch_checked(c, o).await
    }
    async fn search(&self, c: &str, t: &str, l: usize) -> Result<Vec<SearchHit>> {
        self.claimed.search(c, t, l).await
    }
    async fn drop_collection(&self, c: &str) -> Result<()> {
        self.claimed.drop_collection(c).await
    }
    async fn collections(&self, p: &str) -> Result<Vec<String>> {
        self.claimed.collections(p).await
    }
}

#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "claim_bound_tests.rs"]
mod tests;
