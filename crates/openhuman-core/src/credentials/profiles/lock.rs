//! Auth-profile locking: the shared local lease for a profile-local document,
//! or, for a tenant document on a storage backend, the backend lease
//! (`backend.rs`).

use anyhow::Result;
use tinystoragedrivers_fence::{LocalLeaseGuard, LocalLeases};

use super::backend::{self, BackendLease};
use super::AuthProfilesStore;

/// Held for the duration of an auth-profile operation; reads and writes of
/// the document go through the store it was taken on.
pub(super) struct AuthProfileLockGuard {
    held: Held,
}

enum Held {
    /// A local lease, or `None` for the lock-free read-only snapshot.
    Local(Option<LocalLeaseGuard>),
    Backend(BackendLease),
}

impl AuthProfileLockGuard {
    /// No lease: the read-only snapshot of a full or read-only filesystem.
    pub(super) fn lock_free() -> Self {
        Self {
            held: Held::Local(None),
        }
    }

    /// The backend lease, when the document lives on the storage backend.
    pub(super) fn backend(&self) -> Option<&BackendLease> {
        match &self.held {
            Held::Backend(lease) => Some(lease),
            Held::Local(_) => None,
        }
    }
}

impl AuthProfilesStore {
    /// Acquire an exclusive lease for a profile-store mutation.
    pub(super) fn acquire_lock(&self) -> Result<AuthProfileLockGuard> {
        if let Some(document) = backend::document_for(&self.state_dir)? {
            return Ok(AuthProfileLockGuard {
                held: Held::Backend(document.lock()),
            });
        }
        LocalLeases::new(self.lock_path.clone())
            .acquire()
            .map(|lease| AuthProfileLockGuard {
                held: Held::Local(Some(lease)),
            })
            .map_err(|error| anyhow::anyhow!("Failed to acquire auth profile lease: {error}"))
    }

    /// Acquire the read lease, returning `None` only for the TSD-approved
    /// lock-free fallback when a full or read-only filesystem cannot create
    /// the lease file.
    pub(super) fn acquire_read_lock(&self) -> Result<Option<AuthProfileLockGuard>> {
        if let Some(document) = backend::document_for(&self.state_dir)? {
            return Ok(Some(AuthProfileLockGuard {
                held: Held::Backend(document.lock()),
            }));
        }
        LocalLeases::new(self.lock_path.clone())
            .acquire_for_read()
            .map(|lease| {
                lease.map(|lease| AuthProfileLockGuard {
                    held: Held::Local(Some(lease)),
                })
            })
            .map_err(|error| anyhow::anyhow!("Failed to acquire auth profile read lease: {error}"))
    }
}

#[cfg(test)]
#[path = "lock_tests.rs"]
mod tests;
