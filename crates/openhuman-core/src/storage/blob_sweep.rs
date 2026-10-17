//! Periodic reclamation of expired blobs.
//!
//! A blob written with a lifetime reads as absent once it lapses, but its
//! bytes stay in the backend until something sweeps them
//! (`StorageBackend::sweep_expired_blobs`, the trait-level hook the
//! tinystoragedrivers drivers implement). The host owns the schedule: this
//! module visits every storage scope background work covers
//! ([`super::agents::for_each_scope`]) on a fixed tick and asks the installed
//! backend to reclaim that scope's lapsed blobs.
//!
//! It is best-effort maintenance. A scope that fails to sweep is logged and
//! retried on the next tick; nothing here blocks boot or a request.

use std::sync::Arc;
use std::time::Duration;

use tinystoragedrivers::{Scope, StorageBackend};

/// How often the sweeper wakes. Lapsed blobs are already invisible to readers,
/// so this only bounds how long their bytes linger.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Delay before the first sweep, so boot work is not competing with it.
pub const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(5 * 60);

/// The longest one scope's sweep may run. The drivers' sweeps carry no
/// deadline of their own (a remote backend can stall on a query), and scopes
/// are swept one after another, so a hung scope would starve every later one
/// and the next tick. A timed-out scope counts as a failed sweep and is
/// retried on the next tick.
pub const SCOPE_SWEEP_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Reclaims the lapsed blobs of `scope`, returning how many went. A failure
/// or a sweep past [`SCOPE_SWEEP_TIMEOUT`] is logged and counts as zero.
pub async fn sweep_scope(backend: &dyn StorageBackend, scope: &Scope) -> u64 {
    sweep_scope_within(backend, scope, SCOPE_SWEEP_TIMEOUT).await
}

/// [`sweep_scope`] with an explicit deadline.
pub(crate) async fn sweep_scope_within(
    backend: &dyn StorageBackend,
    scope: &Scope,
    limit: Duration,
) -> u64 {
    let outcome = match tokio::time::timeout(limit, backend.sweep_expired_blobs(scope)).await {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!(
                scope = scope.as_str(),
                limit_secs = limit.as_secs_f32(),
                "[storage::blob_sweep] sweep timed out; retrying on the next tick"
            );
            return 0;
        }
    };
    match outcome {
        Ok(removed) => {
            if removed > 0 {
                tracing::debug!(
                    scope = scope.as_str(),
                    removed,
                    "[storage::blob_sweep] reclaimed expired blobs"
                );
            }
            removed
        }
        Err(error) => {
            tracing::warn!(
                scope = scope.as_str(),
                %error,
                "[storage::blob_sweep] sweep failed; retrying on the next tick"
            );
            0
        }
    }
}

/// Reclaims the lapsed blobs of each SaaS profile in `profile_ids`: the
/// profile's own scope (`u:<id>`) and the scopes the blob-owning modules
/// keep for it. Returns the total reclaimed.
pub(crate) async fn sweep_profiles(backend: &dyn StorageBackend, profile_ids: &[String]) -> u64 {
    let mut removed = 0;
    for id in profile_ids {
        let scope = super::scope_for_profile(id);
        removed += sweep_scope(backend, &scope).await;
        for module_scope in crate::modules::expiring_blob_module_scopes_for(id) {
            if module_scope != scope {
                removed += sweep_scope(backend, &module_scope).await;
            }
        }
    }
    removed
}

/// One pass over every scope background work covers, against `backend`:
/// `local` outside SaaS mode, each known agent, and each provisioned SaaS
/// profile (`profile:<id>`, which [`super::agents::for_each_scope`] does not
/// visit). Returns the total number of blobs reclaimed.
///
/// Agent scopes are enumerated from the installed backend (the only caller,
/// [`sweep_installed`], passes that one).
pub async fn sweep_all(backend: Arc<dyn StorageBackend>) -> u64 {
    let scoped: u64 = super::agents::for_each_scope(&crate::core::runtime::DispatchCtx::host_default(), "blob sweep", |ctx| {
        let backend = Arc::clone(&backend);
        async move {
            match super::current_scope() {
                Ok(scope) => {
                    // Modules keep their expiring blobs in their own scopes
                    // derived from the acting scope (TinyJuice's CCR TTL).
                    let mut removed = sweep_scope(backend.as_ref(), &scope).await;
                    for module_scope in crate::modules::expiring_blob_module_scopes(&ctx) {
                        if module_scope != scope {
                            removed += sweep_scope(backend.as_ref(), &module_scope).await;
                        }
                    }
                    removed
                }
                Err(error) => {
                    tracing::debug!(%error, "[storage::blob_sweep] no scope for this visit; skipped");
                    0
                }
            }
        }
    })
    .await
    .into_iter()
    .map(|(_, removed)| removed)
    .sum();
    scoped + sweep_host_profiles(backend.as_ref()).await
}

/// [`sweep_profiles`] over the profiles the process's profile host has
/// provisioned; zero outside SaaS mode (no host installed).
async fn sweep_host_profiles(backend: &dyn StorageBackend) -> u64 {
    let Some(host) = crate::profiles::host::host() else {
        return 0;
    };
    match bounded_listing(SCOPE_SWEEP_TIMEOUT, host.ids()).await {
        Some(Ok(ids)) => sweep_profiles(backend, &ids).await,
        Some(Err(error)) => {
            tracing::warn!(%error, "[storage::blob_sweep] could not list profiles; skipped");
            0
        }
        None => 0,
    }
}

/// Awaits a profile listing for at most `limit`; `None` (logged) when it hangs,
/// so a stuck registry cannot stall the scheduler. Retried on the next tick.
async fn bounded_listing<T>(
    limit: Duration,
    listing: impl std::future::Future<Output = T>,
) -> Option<T> {
    match tokio::time::timeout(limit, listing).await {
        Ok(value) => Some(value),
        Err(_) => {
            tracing::warn!(
                limit_secs = limit.as_secs_f32(),
                "[storage::blob_sweep] profile listing timed out; retrying on the next tick"
            );
            None
        }
    }
}

/// [`sweep_all`] against the installed backend; zero when none is installed.
pub async fn sweep_installed() -> u64 {
    match super::installed() {
        Some(backend) => sweep_all(backend).await,
        None => 0,
    }
}

/// Spawns the sweeper: after `first_delay`, then every `interval`, until the
/// task is aborted. Missed ticks are skipped, not replayed.
pub fn spawn(
    ctx: &crate::core::runtime::DispatchCtx,
    first_delay: Duration,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    ctx.spawn(async move {
        tracing::debug!(
            first_delay_secs = first_delay.as_secs(),
            interval_secs = interval.as_secs(),
            "[storage::blob_sweep] scheduled"
        );
        tokio::time::sleep(first_delay).await;
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let removed = sweep_installed().await;
            tracing::trace!(removed, "[storage::blob_sweep] tick complete");
        }
    })
}

#[cfg(test)]
#[path = "blob_sweep_tests.rs"]
mod tests;
