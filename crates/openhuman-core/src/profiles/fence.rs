//! A profile's lease fence: which scopes its grant guards, and how the host
//! registers, renews and retires it.
//!
//! When [`ProfileHost::open`](super::ProfileHost::open) takes a profile's
//! lease it builds a [`LeaseFence`] for the grant and registers it with the
//! host's [`FenceRegistry`] (the process registry, which every backend from
//! `storage::open` consults). From then on each write through the storage
//! ports to one of the profile's scopes is checked against the grant:
//!
//! - the profile's tenant scope (`storage::scope_for_profile`, `u:<id>`),
//!   where its domain stores, secrets, cron and flows keep their records;
//! - every principal scope under the tenant (`u:<id>/agent:<agent>` and
//!   `u:<id>/module:<module>`), where the session store keeps transcripts and
//!   turn states and modules keep theirs.
//!
//! The tenant is the fence boundary: one exact scope plus the prefix
//! `<tenant>/`, which a hashed segment cannot escape (tsd hashes the segment,
//! never the whole scope).
//!
//! The heartbeat records each renewal on the fence and latches it when the
//! lease is lost; a write check that finds the record moved on latches it
//! too. A latched fence stays registered, refusing writes, for as long as
//! the fenced profile's context is alive (in-flight work holds it), and is
//! replaced when this node opens the profile again. A clean release (idle
//! eviction, `profiles.release`, shutdown) retires it.
//!
//! # File leases
//!
//! Without a storage backend the host uses `LocalLeases` (an `flock`). That
//! lease cannot be lost while the process lives: nobody else can take the
//! lock, and the OS drops it only when the holder dies. Nothing goes
//! through the storage ports either (no backend is installed), so the fence
//! is registered but never consulted by a write; `ensure_hosted` and the
//! background loop still read its latch. Two hosts on one root are kept
//! apart by the lock itself, and only on a filesystem where `flock` works
//! (not most network filesystems).

use std::sync::Arc;
use std::time::Duration;

use crate::storage::fence::{LeaseFence, ScopeMatch};
use crate::storage::lease::{LeaseGrant, LeaseStore};

use super::types::ProfileId;

/// How long before its expiry a grant stops admitting writes: a sixth of the
/// TTL (5 s of the default 30 s), the clock skew the cluster tolerates
/// between nodes. Renewals run every third of the TTL, so a healthy holder
/// never gets near it.
pub(crate) fn skew_margin(ttl: Duration) -> u64 {
    u64::try_from((ttl / 6).as_millis()).unwrap_or(u64::MAX)
}

/// The scopes profile `id`'s grant guards: its tenant scope and everything
/// under `<tenant>/`.
pub(crate) fn scopes_of(id: &ProfileId) -> Vec<ScopeMatch> {
    let tenant = crate::storage::scope_for_profile(id.as_str());
    vec![
        ScopeMatch::Exact(tenant.as_str().to_string()),
        ScopeMatch::Prefix(format!("{}/", tenant.as_str())),
    ]
}

/// A fence for `node`'s `grant` on profile `id`.
pub(crate) fn for_grant(
    leases: Arc<dyn LeaseStore>,
    node: &str,
    id: &ProfileId,
    grant: &LeaseGrant,
    ttl: Duration,
) -> Arc<LeaseFence> {
    Arc::new(LeaseFence::new(
        leases,
        id.as_str(),
        node,
        grant.epoch,
        grant.expires_at_ms,
        skew_margin(ttl),
        scopes_of(id),
    ))
}

#[cfg(test)]
#[path = "fence_tests.rs"]
mod tests;
