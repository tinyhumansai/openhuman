//! Epoch fencing for writes to leased storage scopes.
//!
//! A lease ([`super::lease`]) says which node owns a key; it does not stop a
//! node that lost the key from writing. A paused or partitioned holder keeps
//! running its in-process work until its heartbeat notices the loss, and in
//! that window its writes would land on top of the new holder's.
//!
//! A [`LeaseFence`] closes that window for the storage ports. It is one
//! grant (key, node, epoch) plus the scopes that grant protects. While a
//! fence is registered in a [`FenceRegistry`], every write a
//! [`FencedBackend`](super::fenced_backend::FencedBackend) sends to one of
//! those scopes is checked twice.
//!
//! First, on this node, [`LeaseFence::check`] refuses the write with a typed
//! [`FenceError`] when:
//!
//! 1. the fence is latched ([`LeaseFence::fence`]: the heartbeat lost the
//!    lease, or an earlier check saw it superseded);
//! 2. the grant ran out by this node's clock, less a skew margin
//!    ([`LeaseFence::check_local`], no I/O); or
//! 3. the stored lease record no longer names this node at this epoch, or
//!    has expired, or cannot be read (fail closed).
//!
//! Then the storage driver enforces it. The write goes through
//! [`StorageBackend::for_scope_fenced`](tinystoragedrivers::StorageBackend::for_scope_fenced)
//! with [`LeaseFence::driver_fence`], so the driver re-reads the lease record
//! in the same atomic step as the write and refuses it
//! ([`ErrorKind::Fenced`](tinystoragedrivers::ErrorKind::Fenced)) unless the
//! record still names this node at this epoch, unreleased. A holder that
//! passes the first check and is then paused past a takeover cannot land
//! the write: the driver refusal latches the fence and surfaces as
//! [`FenceError::Superseded`].
//!
//! # Where the race is still open
//!
//! The driver step needs [`Capability::Fencing`](tinystoragedrivers::Capability::Fencing)
//! and a lease record in the storage ports. Without either, the write runs
//! after the first check alone, and a holder paused for longer than its
//! remaining grant *after* a successful check can still land one write after
//! a takeover:
//!
//! - MongoDB without transactions (a standalone server) has no fencing, and
//!   MongoDB blob writes cannot be fenced (GridFS cannot join a
//!   transaction);
//! - writes to a named database ([`StorageBackend::database`](tinystoragedrivers::StorageBackend::database))
//!   cannot see the lease record, which lives in the default database;
//! - file leases (hosts without a backend) keep no record in the ports, but
//!   they cannot be lost while the process lives either.
//!
//! The skew margin makes that race need a pause longer than the margin
//! after a successful check, rather than any pause at all.
//!
//! Scopes nobody registered pass through untouched, so a backend with an
//! empty registry (every single-user host) behaves exactly as before.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use tinystoragedrivers_types::WriteCondition;

use super::lease::LeaseStore;
use super::StorageError;

/// Why a fenced write was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FenceError {
    /// This node lost the lease (latched).
    #[error("lease fence: key lost at epoch {epoch}; this node no longer holds it")]
    Fenced {
        /// The epoch this node held.
        epoch: u64,
    },
    /// The grant ran out by this node's clock (less the skew margin) before
    /// it was renewed.
    #[error("lease fence: grant at epoch {epoch} expired before renewal")]
    Expired {
        /// The epoch this node holds.
        epoch: u64,
    },
    /// The stored record names another holder or a newer epoch.
    #[error("lease fence: epoch {held} superseded (stored epoch {stored:?})")]
    Superseded {
        /// The epoch this node held.
        held: u64,
        /// The stored record's epoch, if there is a record.
        stored: Option<u64>,
    },
    /// The lease record could not be read, so the write is refused.
    #[error("lease fence: could not verify the lease: {0}")]
    Unverified(String),
}

impl FenceError {
    /// This refusal as a port error ([`ErrorKind::Backend`](tinystoragedrivers::ErrorKind),
    /// not retryable, so compare-and-swap loops do not spin on it) carrying
    /// `self` as its source; [`fence_error`] recovers it.
    pub fn into_storage(self) -> StorageError {
        StorageError::backend(self.to_string()).with_source(self)
    }
}

/// The [`FenceError`] behind `error`, when a fence refused the write.
pub fn fence_error(error: &StorageError) -> Option<&FenceError> {
    std::error::Error::source(error).and_then(|source| source.downcast_ref::<FenceError>())
}

/// Which scopes a fence protects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMatch {
    /// Exactly this scope.
    Exact(String),
    /// Every scope starting with this prefix.
    Prefix(String),
}

impl ScopeMatch {
    fn covers(&self, scope: &str) -> bool {
        match self {
            Self::Exact(exact) => scope == exact,
            Self::Prefix(prefix) => scope.starts_with(prefix.as_str()),
        }
    }
}

/// One node's grant on a key, guarding the scopes it protects. See the
/// module docs.
pub struct LeaseFence {
    key: String,
    node: String,
    epoch: u64,
    expires_at_ms: AtomicU64,
    margin_ms: u64,
    scopes: Vec<ScopeMatch>,
    /// Weak: a fence must not keep its node's lease store (and so a file
    /// lock) alive after the host that owns it is gone. A gone store fails
    /// the check closed.
    leases: Weak<dyn LeaseStore>,
    latch: tokio::sync::watch::Sender<bool>,
    /// While the fence is latched, it stays registered (refusing writes)
    /// until this anchor is gone: the work that could still write is what
    /// holds it.
    anchor: Mutex<Option<Weak<dyn Any + Send + Sync>>>,
}

impl std::fmt::Debug for LeaseFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeaseFence")
            .field("node", &self.node)
            .field("epoch", &self.epoch)
            .field("fenced", &self.is_fenced())
            .finish_non_exhaustive()
    }
}

impl LeaseFence {
    /// A fence for `node`'s grant on `key` at `epoch`, valid until
    /// `expires_at_ms` less `margin_ms`, verified against `leases`.
    pub fn new(
        leases: Arc<dyn LeaseStore>,
        key: impl Into<String>,
        node: impl Into<String>,
        epoch: u64,
        expires_at_ms: u64,
        margin_ms: u64,
        scopes: Vec<ScopeMatch>,
    ) -> Self {
        Self {
            key: key.into(),
            node: node.into(),
            epoch,
            expires_at_ms: AtomicU64::new(expires_at_ms),
            margin_ms,
            scopes,
            leases: Arc::downgrade(&leases),
            latch: tokio::sync::watch::channel(false).0,
            anchor: Mutex::new(None),
        }
    }

    /// The leased key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The epoch this fence guards with.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The node holding the grant.
    pub fn node(&self) -> &str {
        &self.node
    }

    /// The condition a storage driver enforces for this grant: the lease record
    /// (where the lease store keeps it, [`LeaseStore::write_condition`]) must
    /// still name this node at this epoch, unreleased. `None` when the lease
    /// store keeps no record in the storage ports (file leases) or is gone;
    /// then only [`Self::check`] guards the write.
    pub fn write_condition(&self) -> Option<WriteCondition> {
        self.leases
            .upgrade()?
            .write_condition(&self.key, &self.node, self.epoch)
    }

    /// The refusal for a write the storage driver fenced off
    /// ([`ErrorKind::Fenced`](tinystoragedrivers::ErrorKind::Fenced)): the
    /// lease record moved on between [`Self::check`] and the write. Re-reads
    /// the record for the stored epoch; the fence is latched either way.
    pub async fn refused_by_driver(&self, now_ms: u64) -> FenceError {
        match self.check(now_ms).await {
            // `check` latched it if the record moved on.
            Err(error) => {
                self.fence();
                error
            }
            Ok(()) => {
                // The driver could not match a record this node can still
                // read as its own: a guard the driver cannot see. Fail closed.
                tracing::error!(
                    target: "openhuman::storage::fence",
                    epoch = self.epoch,
                    "[storage][fence] driver refused a write the lease record still admits; latching"
                );
                self.fence();
                FenceError::Superseded {
                    held: self.epoch,
                    stored: None,
                }
            }
        }
    }

    /// Whether this fence guards `scope`.
    pub fn covers(&self, scope: &str) -> bool {
        self.scopes.iter().any(|m| m.covers(scope))
    }

    /// Records a renewal: the grant now lasts until `expires_at_ms`.
    pub fn renewed(&self, expires_at_ms: u64) {
        self.expires_at_ms.store(expires_at_ms, Ordering::SeqCst);
    }

    /// Latches the fence: every later check and write is refused. Idempotent.
    pub fn fence(&self) {
        if !self.latch.send_replace(true) {
            tracing::warn!(
                target: "openhuman::storage::fence",
                epoch = self.epoch,
                "[storage][fence] latched; writes to the fenced scopes are refused"
            );
        }
    }

    /// Whether the fence is latched.
    pub fn is_fenced(&self) -> bool {
        *self.latch.borrow()
    }

    /// Resolves once the fence latches (at once when it already has), so
    /// in-process work can stop promptly.
    pub async fn fenced(&self) {
        let mut rx = self.latch.subscribe();
        // `wait_for` only fails when the sender is gone, and `self` owns it.
        let _ = rx.wait_for(|fenced| *fenced).await;
    }

    /// Keeps this fence registered after it latches only for as long as
    /// `anchor` is alive (see [`FenceRegistry::lookup`]). Without an anchor
    /// a latched fence stays until it is retired or replaced.
    pub fn anchor_to(&self, anchor: Weak<dyn Any + Send + Sync>) {
        *self.anchor.lock().unwrap_or_else(PoisonError::into_inner) = Some(anchor);
    }

    fn anchor_alive(&self) -> bool {
        self.anchor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_none_or(|anchor| anchor.strong_count() > 0)
    }

    fn has_dead_anchor(&self) -> bool {
        self.anchor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|anchor| anchor.strong_count() == 0)
    }

    /// The checks that need no I/O: the latch, and the grant's expiry by
    /// this node's clock less the margin.
    ///
    /// # Errors
    ///
    /// [`FenceError::Fenced`] or [`FenceError::Expired`].
    pub fn check_local(&self, now_ms: u64) -> Result<(), FenceError> {
        if self.is_fenced() {
            return Err(FenceError::Fenced { epoch: self.epoch });
        }
        let deadline = self
            .expires_at_ms
            .load(Ordering::SeqCst)
            .saturating_sub(self.margin_ms);
        if now_ms >= deadline {
            return Err(FenceError::Expired { epoch: self.epoch });
        }
        Ok(())
    }

    /// [`Self::check_local`], then the stored record: it must still name
    /// this node at this epoch, unreleased and unexpired. A record that moved
    /// on latches the fence.
    ///
    /// # Errors
    ///
    /// Any [`FenceError`]; a lease store failure is
    /// [`FenceError::Unverified`] (the write is refused, not risked).
    pub async fn check(&self, now_ms: u64) -> Result<(), FenceError> {
        self.check_local(now_ms)?;
        let Some(leases) = self.leases.upgrade() else {
            return Err(FenceError::Unverified("the lease store is gone".into()));
        };
        let record = leases
            .holder(&self.key)
            .await
            .map_err(|error| FenceError::Unverified(error.to_string()))?;
        match record {
            Some(record)
                if record.owner == self.node && record.epoch == self.epoch && !record.released =>
            {
                if now_ms >= record.expires_at_ms {
                    return Err(FenceError::Expired { epoch: self.epoch });
                }
                Ok(())
            }
            other => {
                let stored = other.map(|record| record.epoch);
                tracing::warn!(
                    target: "openhuman::storage::fence",
                    held = self.epoch,
                    stored = ?stored,
                    "[storage][fence] lease superseded; refusing the write"
                );
                self.fence();
                Err(FenceError::Superseded {
                    held: self.epoch,
                    stored,
                })
            }
        }
    }
}

/// The fences a process holds. The process has one ([`registry`]); tests
/// make their own so they never see each other's.
pub struct FenceRegistry {
    fences: Mutex<Vec<Arc<LeaseFence>>>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for FenceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FenceRegistry")
            .field("fences", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl FenceRegistry {
    /// A registry reading `clock` (milliseconds since the epoch) for checks.
    pub fn new(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            fences: Mutex::new(Vec::new()),
            clock: Box::new(clock),
        }
    }

    /// The registry's clock.
    pub fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Arc<LeaseFence>>> {
        self.fences.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `fence`, replacing any fence on the same key unless that
    /// one is from a newer epoch. Fences with dead anchors are pruned first;
    /// newer live or unanchored fences retain their epoch protection.
    pub fn register(&self, fence: Arc<LeaseFence>) {
        let mut fences = self.lock();
        // A dead anchor means no work can still write through the old grant.
        // Prune it before comparing epochs because a fresh lease store may
        // restart its epoch counter while a stale registry entry remains.
        fences.retain(|held| !held.has_dead_anchor());
        // A fence from a newer epoch is never displaced by an older grant's.
        if fences
            .iter()
            .any(|held| held.key == fence.key && held.epoch > fence.epoch)
        {
            tracing::debug!(
                target: "openhuman::storage::fence",
                epoch = fence.epoch,
                "[storage][fence] stale registration ignored"
            );
            return;
        }
        fences.retain(|held| held.key != fence.key);
        tracing::debug!(
            target: "openhuman::storage::fence",
            epoch = fence.epoch,
            "[storage][fence] registered"
        );
        fences.push(fence);
    }

    /// Drops the fence on `key` at `epoch` (a clean release); a fence
    /// registered since under another epoch stays. Cached handles retain the
    /// old fence, so latch it before removing it from the registry.
    pub fn retire(&self, key: &str, epoch: u64) {
        let mut fences = self.lock();
        fences.retain(|held| {
            if held.key == key && held.epoch == epoch {
                held.fence();
                false
            } else {
                true
            }
        });
    }

    /// The fence guarding `scope`, if any. A latched fence whose anchor is
    /// gone (nothing that could still write remains) is pruned here.
    pub fn lookup(&self, scope: &str) -> Option<Arc<LeaseFence>> {
        let mut fences = self.lock();
        fences.retain(|held| !held.is_fenced() || held.anchor_alive());
        fences.iter().find(|held| held.covers(scope)).cloned()
    }

    /// Refuses a write to `scope` unless its fence (when it has one) passes
    /// [`LeaseFence::check`].
    ///
    /// # Errors
    ///
    /// The fence's refusal, as a port error ([`FenceError::into_storage`]).
    pub async fn guard(&self, scope: &str, op: &'static str) -> Result<(), StorageError> {
        self.admit(scope, op).await.map(|_| ())
    }

    /// [`Self::guard`], handing back the fence that admitted the write (if
    /// any) so the caller can have the driver enforce it too.
    ///
    /// # Errors
    ///
    /// The fence's refusal, as a port error ([`FenceError::into_storage`]).
    pub async fn admit(
        &self,
        scope: &str,
        op: &'static str,
    ) -> Result<Option<Arc<LeaseFence>>, StorageError> {
        let Some(fence) = self.lookup(scope) else {
            return Ok(None);
        };
        match fence.check(self.now_ms()).await {
            Ok(()) => Ok(Some(fence)),
            Err(error) => {
                tracing::warn!(
                    target: "openhuman::storage::fence",
                    op,
                    epoch = fence.epoch,
                    "[storage][fence] refused a write: {error}"
                );
                Err(error.into_storage())
            }
        }
    }

    /// The port error for a write to `fence`'s scopes that the storage
    /// driver refused ([`LeaseFence::refused_by_driver`]).
    pub async fn driver_refused(&self, fence: &LeaseFence, op: &'static str) -> StorageError {
        let error = fence.refused_by_driver(self.now_ms()).await;
        tracing::warn!(
            target: "openhuman::storage::fence",
            op,
            epoch = fence.epoch,
            "[storage][fence] the driver refused a write the host check admitted: {error}"
        );
        error.into_storage()
    }
}

/// Milliseconds since the Unix epoch.
pub fn system_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

static REGISTRY: LazyLock<Arc<FenceRegistry>> =
    LazyLock::new(|| Arc::new(FenceRegistry::new(system_now_ms)));

/// The process's fence registry, which [`super::open`]'s backends consult.
pub fn registry() -> Arc<FenceRegistry> {
    Arc::clone(&REGISTRY)
}

#[cfg(test)]
#[path = "fence_tests.rs"]
mod tests;
