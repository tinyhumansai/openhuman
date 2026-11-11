//! Auth-profile locking through the shared local lease implementation.

use anyhow::Result;
use tinystoragedrivers_fence::{LocalLeaseGuard, LocalLeases};

use super::AuthProfilesStore;

/// Owns one TSD local lease for the duration of an auth-profile operation.
pub(super) struct AuthProfileLockGuard {
    _lease: LocalLeaseGuard,
}

impl AuthProfilesStore {
    /// Acquire an exclusive lease for a profile-store mutation.
    pub(super) fn acquire_lock(&self) -> Result<AuthProfileLockGuard> {
        LocalLeases::new(self.lock_path.clone())
            .acquire()
            .map(|lease| AuthProfileLockGuard { _lease: lease })
            .map_err(|error| anyhow::anyhow!("Failed to acquire auth profile lease: {error}"))
    }

    /// Acquire the read lease, returning `None` only for the TSD-approved
    /// lock-free fallback when a full or read-only filesystem cannot create
    /// the lease file.
    pub(super) fn acquire_read_lock(&self) -> Result<Option<AuthProfileLockGuard>> {
        LocalLeases::new(self.lock_path.clone())
            .acquire_for_read()
            .map(|lease| lease.map(|lease| AuthProfileLockGuard { _lease: lease }))
            .map_err(|error| anyhow::anyhow!("Failed to acquire auth profile read lease: {error}"))
    }
}

#[cfg(test)]
#[path = "lock_tests.rs"]
mod tests;
