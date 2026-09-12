//! Where Lane C reads from.
//!
//! The lane is written against a two-method trait rather than the guard
//! directly so the turn tests can script hits without a memory driver, and so
//! the lane's own tests can exercise the timeout and error paths on demand.
//! Production binds [`GuardSource`], which goes through the bound driver's
//! guard: scope narrowing, taint stamping and the recall budget all apply
//! exactly as they do for the memory tools.
//!
//! Both methods are required, neither defaulted. A default answering "nothing"
//! would compile for every implementor and be a silent no-op in production —
//! the exact shape of #6041, where a `Memory` trait default returned `Ok(vec![])`
//! under every module-backed install while every test stayed green.

use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::retrieval::{FastRetrieveQuery, RetrievalResponse};
use crate::openhuman::memory::api::provider::MemoryProvider as _;
use crate::openhuman::memory::api::types::NamespaceMemoryHit;
use crate::openhuman::memory::guard::MemoryGuard;
use crate::openhuman::memory::source_scope::as_bus_scope;
use async_trait::async_trait;
use std::sync::Arc;

/// The retrievals the lane can run: the tree walk, and the scored recall over
/// one namespace (#6063).
#[async_trait]
pub trait AutoRecallSource: Send + Sync {
    /// The driver's `fast_retrieve` for `query`, bounded by `options`.
    async fn fast_retrieve(
        &self,
        query: &str,
        options: FastRetrieveQuery,
    ) -> Result<RetrievalResponse, MemoryError>;

    /// The driver's scored recall over `namespace` for `query`: at most `limit`
    /// hits, each carrying the vector similarity the lane floors on.
    async fn recall_namespace_scored(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError>;
}

/// The production source: the session's bound driver, behind its guard.
pub struct GuardSource {
    guard: Arc<MemoryGuard>,
}

impl GuardSource {
    /// A source over `guard`.
    pub fn new(guard: Arc<MemoryGuard>) -> Self {
        Self { guard }
    }
}

#[async_trait]
impl AutoRecallSource for GuardSource {
    async fn fast_retrieve(
        &self,
        query: &str,
        options: FastRetrieveQuery,
    ) -> Result<RetrievalResponse, MemoryError> {
        // A driver without the retrieval family has no tree to rank. That is a
        // degradation, not an error: the lane yields nothing, the same footing
        // Lane B's `recall_by_vector` takes for a driver without vectors.
        let Some(retrieval) = self.guard.as_retrieval() else {
            log::debug!(
                "[auto_recall] bound driver exposes no retrieval family; nothing to recall"
            );
            return Ok(RetrievalResponse::default());
        };
        // The per-turn source allowlist is host policy and a task-local; it
        // does not cross the bus, so it is passed explicitly. The guard
        // narrows it further and never widens it.
        let scope = as_bus_scope();
        retrieval
            .fast_retrieve(query, options, scope.as_ref())
            .await
    }

    async fn recall_namespace_scored(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        // Same degradation as the tree leg: no retrieval family, no notes.
        let Some(retrieval) = self.guard.as_retrieval() else {
            log::debug!(
                "[auto_recall] bound driver exposes no retrieval family; no notes to recall"
            );
            return Ok(Vec::new());
        };
        // No session to exclude: the notes namespace is never auto-saved per
        // session, and the lane runs before this turn is archived, so there is
        // no self-echo for the engine's exclusion to catch.
        retrieval
            .recall_namespace_scored(namespace, query, limit, None)
            .await
    }
}
