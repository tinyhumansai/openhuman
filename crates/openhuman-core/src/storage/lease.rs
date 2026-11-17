//! Exclusive, expiring ownership of a named key: the lease.
//!
//! A lease says which node owns a key (in SaaS mode: which core process
//! serves a profile) and until when. Two implementations share the
//! [`LeaseStore`] contract:
//!
//! - [`DocumentLeases`] keeps one host-owned record per key in
//!   [`LEASE_COLLECTION`] under [`CLUSTER_SCOPE`], using versioned document
//!   writes as a compare-and-swap so it is correct across processes on a
//!   driver with cross-process CAS ([`super::driver_has_cross_process_cas`]).
//! - [`LocalLeases`] is for hosts without a backend: an exclusive `flock` on
//!   `<root>/<sha256(key) hex>/.lease`, which the OS drops when the holder dies.
//!
//! No wall clock is read here: every operation takes `now_ms`, so behaviour
//! is a function of its inputs and the tests are deterministic.
//!
//! # Semantics
//!
//! - **acquire** takes the key when it has no record, its record is
//!   released, expired (`now_ms >= expires_at_ms`), or already this node's.
//!   Otherwise it fails with [`LeaseError::Held`] carrying the live record.
//!   A fresh key starts at epoch 1; every acquisition other than a
//!   re-entrant one (this same store instance already holds that epoch)
//!   writes `epoch + 1`, so epochs only grow. An expired takeover does not
//!   stop the previous holder: resources a lease protects must reject writes
//!   carrying an epoch below the current one, and a holder must stop work
//!   before `expires_at_ms` minus the maximum clock skew between nodes (each
//!   node supplies its own `now_ms`). This module does not check epochs at
//!   the protected resource; [`super::fence`] does, for storage-port writes. An epoch at `u64::MAX` cannot be advanced, so a
//!   takeover of it fails with a storage error instead of repeating it.
//! - **renew** extends a grant while it is the latest one its holder was
//!   issued at that epoch and the stored record still names its holder and
//!   epoch. Any takeover, release or newer grant since makes it
//!   [`LeaseError::Lost`].
//! - **release** marks the record `released = true` and keeps it, so the next
//!   acquirer can tell a clean hand-over from a crash.
//!
//! # `previous_unclean`
//!
//! A grant's `previous_unclean` is `true` exactly when the acquisition
//! replaced a record that was **not released** and that this store instance
//! did not itself hold. That covers a different owner whose lease expired
//! (it stopped renewing: crashed, partitioned or hung) and this node's own
//! id left over from an earlier process (it crashed before releasing). A
//! fresh key, a released record and a re-entrant acquire are clean. The
//! profile host runs workspace recovery when it is set.
//!
//! Node ids must be unique among live processes: a record carrying this
//! node's id but not held by this instance is treated as a dead previous
//! incarnation and taken over at once.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tinystoragedrivers::Version;

use super::StorageError;

pub use super::lease_documents::{DocumentLeases, LEASE_OWNER};
pub use super::lease_local::LocalLeases;

/// The storage scope lease records live under, shared by every node.
pub const CLUSTER_SCOPE: &str = "cluster";

/// The host-owned collection for process-placement lease records.
pub const LEASE_COLLECTION: &str = "openhuman.leases";

/// The longest key a lease accepts, in bytes.
pub const MAX_KEY_LEN: usize = 200;

/// The stored state of one key's lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    /// The node that holds (or last held) the key.
    pub owner: String,
    /// Where that node can be reached, for redirecting callers.
    pub endpoint: Option<String>,
    /// Bumped on every acquisition that is not re-entrant.
    pub epoch: u64,
    /// The lease is void from this instant (milliseconds) on.
    pub expires_at_ms: u64,
    /// The holder let go cleanly.
    pub released: bool,
}

impl LeaseRecord {
    /// Whether the record still excludes other nodes at `now_ms`.
    #[must_use]
    pub fn is_live(&self, now_ms: u64) -> bool {
        !self.released && now_ms < self.expires_at_ms
    }

    /// Milliseconds until the record stops excluding others (0 when it
    /// already does not).
    #[must_use]
    pub fn retry_after_ms(&self, now_ms: u64) -> u64 {
        if self.released {
            0
        } else {
            self.expires_at_ms.saturating_sub(now_ms)
        }
    }
}

/// Proof that the caller holds a key, as of its last acquire or renew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseGrant {
    /// The leased key.
    pub key: String,
    /// The epoch this grant fences with.
    pub epoch: u64,
    /// Which grant of this epoch it is: every acquire or renew by the holder
    /// issues the next one, and only the latest can renew or release.
    pub version: Version,
    /// The grant is void from this instant (milliseconds) on unless renewed.
    pub expires_at_ms: u64,
    /// The acquisition replaced a record nobody released (see the module
    /// docs): the previous holder's work may be half done.
    pub previous_unclean: bool,
}

/// Why a lease operation failed.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    /// Another node holds the key.
    #[error("lease held by {} (epoch {})", .0.owner, .0.epoch)]
    Held(LeaseRecord),
    /// The grant is no longer the current one: the key was taken over or
    /// released since.
    #[error("lease lost")]
    Lost,
    /// The backend failed, or the key is invalid.
    #[error("lease storage: {0}")]
    Storage(#[from] StorageError),
}

/// Exclusive, expiring ownership of named keys. See the module docs.
#[async_trait]
pub trait LeaseStore: Send + Sync {
    /// Takes `key` for this node.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Held`] when another node's record is live, or
    /// [`LeaseError::Storage`].
    async fn acquire(&self, key: &str, now_ms: u64) -> Result<LeaseGrant, LeaseError>;

    /// Extends `grant` to `now_ms` plus the store's TTL.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Lost`] when the record changed since `grant`, or
    /// [`LeaseError::Storage`].
    async fn renew(&self, grant: &LeaseGrant, now_ms: u64) -> Result<LeaseGrant, LeaseError>;

    /// Lets go of `grant`, leaving a released record behind.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Lost`] when the record changed since `grant`, or
    /// [`LeaseError::Storage`].
    async fn release(&self, grant: LeaseGrant) -> Result<(), LeaseError>;

    /// The stored record for `key`, live or not; check
    /// [`LeaseRecord::is_live`].
    ///
    /// # Errors
    ///
    /// [`LeaseError::Storage`].
    async fn holder(&self, key: &str) -> Result<Option<LeaseRecord>, LeaseError>;

    /// Reads the stored holder without declaring schema or creating local
    /// lease files. Implementations that cannot guarantee a read-only lookup
    /// fail closed.
    async fn holder_readonly(&self, _key: &str) -> Result<Option<LeaseRecord>, LeaseError> {
        Err(LeaseError::Storage(StorageError::backend(
            "lease store does not support read-only holder inspection",
        )))
    }

    /// The condition a storage driver evaluates for `node`'s grant on `key` at
    /// `epoch`, when the record lives in the storage ports at all: the
    /// record must still name `node` at `epoch`, unreleased. The driver then
    /// checks a write against the record in the same atomic step
    /// ([`super::fence::LeaseFence::write_condition`]). `None` (the default) for
    /// stores that keep records elsewhere, such as file locks.
    fn write_condition(
        &self,
        _key: &str,
        _node: &str,
        _epoch: u64,
    ) -> Option<tinystoragedrivers_types::WriteCondition> {
        None
    }
}

/// Rejects keys that could escape a directory or a document id: empty,
/// too long, `.`-led, or containing anything but ASCII letters, digits and
/// `-`, `_`, `.`, `@`.
///
/// # Errors
///
/// [`StorageError`] of kind invalid input.
pub fn validate_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return Err(StorageError::invalid_input(format!(
            "lease key must be 1..={MAX_KEY_LEN} bytes"
        )));
    }
    if key.starts_with('.') {
        return Err(StorageError::invalid_input(
            "lease key must not start with '.'",
        ));
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
    {
        return Err(StorageError::invalid_input(
            "lease key may hold only ASCII letters, digits and - _ . @",
        ));
    }
    Ok(())
}

/// What an acquire does with the record it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Takeover {
    /// Write this epoch; `unclean` is the grant's `previous_unclean`.
    Take { epoch: u64, unclean: bool },
    /// Someone else's live record stands.
    Refuse,
    /// The record's epoch cannot be advanced.
    Exhausted,
}

/// The acquire rule both stores share. `held_epoch` is the epoch this store
/// instance currently holds for the key, if any.
pub(crate) fn decide(
    found: Option<&LeaseRecord>,
    node: &str,
    held_epoch: Option<u64>,
    now_ms: u64,
) -> Takeover {
    let Some(record) = found else {
        return Takeover::Take {
            epoch: 1,
            unclean: false,
        };
    };
    let live_reentrant = record.owner == node
        && held_epoch == Some(record.epoch)
        && !record.released
        && now_ms < record.expires_at_ms;
    if live_reentrant {
        return Takeover::Take {
            epoch: record.epoch,
            unclean: false,
        };
    }
    let Some(next) = record.epoch.checked_add(1) else {
        return Takeover::Exhausted;
    };
    if record.released {
        return Takeover::Take {
            epoch: next,
            unclean: false,
        };
    }
    if record.owner == node {
        return Takeover::Take {
            epoch: next,
            unclean: true,
        };
    }
    if now_ms >= record.expires_at_ms {
        return Takeover::Take {
            epoch: next,
            unclean: true,
        };
    }
    Takeover::Refuse
}

/// The directory a [`LocalLeases`] keeps `key`'s lock and record in.
///
/// The directory name is the SHA-256 of the key, so keys that differ only by
/// case, trailing dots or reserved device names never alias on
/// case-insensitive or Windows filesystems.
pub(crate) fn local_dir(root: &std::path::Path, key: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    root.join(hex::encode(Sha256::digest(key.as_bytes())))
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lease_model_tests.rs"]
mod lease_model_tests;
