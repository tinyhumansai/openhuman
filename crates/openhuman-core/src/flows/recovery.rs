//! Host entry points for TinyFlows-owned run recovery.
//!
//! Run rows, lifecycle compare-and-swaps, checkpoints, dedup settlement, and
//! terminal run events belong to the TinyFlows Runs module. OpenHuman supplies
//! only the scope-bound CatalogStorage callback and projects the module's
//! changed runs through the shared runs adapter.

use std::sync::Arc;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::flows::{catalog_storage::CatalogStorageRegistration, runs_adapter};

/// Expire parked runs in this dispatch scope, preserving the read-path's
/// best-effort behavior. The module returns only rows whose parked-only expiry
/// compare-and-swap won; the runs adapter projects and releases those rows.
pub(crate) async fn expire_parked_runs(ctx: &DispatchCtx, config: &Config) -> usize {
    let storage = match crate::flows::catalog_storage::register(ctx, config).await {
        Ok(storage) => storage,
        Err(error) => {
            tracing::warn!(target: "flows", %error, "[flows] parked-run TTL sweep could not register CatalogStorage; read continues");
            return 0;
        }
    };

    match runs_adapter::expire_parked(ctx, config, storage).await {
        Ok(response) => response.expired.len(),
        Err(error) => {
            tracing::warn!(target: "flows", %error, "[flows] parked-run TTL sweep failed; read continues");
            0
        }
    }
}

/// Reconcile orphaned local runs across the selected storage scopes.
///
/// A shared backend is deliberately left alone because a `running` row can be
/// owned by another replica. Each local scope gets its own stable storage
/// callback. The module applies the boot floor and live-actor/lease checks and
/// returns only rows it changed; the runs adapter handles event projection and
/// releases retained callback state for those rows.
pub(crate) async fn sweep_orphaned_runs_on_boot(ctx: &DispatchCtx, config: &Config) -> usize {
    sweep_scopes(
        ctx,
        crate::storage::installed_is_shared(),
        |scope_ctx| async move { sweep_one_scope(&scope_ctx, config).await },
    )
    .await
}

/// Visit every storage scope with `sweep_one`, unless the backend is shared:
/// the module would skip each scope anyway, but registering a scope's storage
/// callback first prepares its collections, which on a host with many agents
/// is the whole cost of boot.
async fn sweep_scopes<F, Fut>(ctx: &DispatchCtx, shared: bool, sweep_one: F) -> usize
where
    F: Fn(DispatchCtx) -> Fut,
    Fut: std::future::Future<Output = usize>,
{
    if shared {
        tracing::info!(target: "flows", "[flows] boot sweep skipped: the storage backend is shared");
        return 0;
    }
    crate::storage::agents::for_each_scope(ctx, "flows boot sweep", sweep_one)
        .await
        .into_iter()
        .map(|(_, swept)| swept)
        .sum()
}

async fn sweep_one_scope(ctx: &DispatchCtx, config: &Config) -> usize {
    let storage: Arc<CatalogStorageRegistration> = match crate::flows::catalog_storage::register(
        ctx, config,
    )
    .await
    {
        Ok(storage) => storage,
        Err(error) => {
            tracing::warn!(target: "flows", %error, "[flows] boot sweep: CatalogStorage registration failed");
            return 0;
        }
    };

    match runs_adapter::boot_sweep(ctx, config, storage).await {
        Ok(response) => response.interrupted.len(),
        Err(error) => {
            tracing::warn!(target: "flows", %error, "[flows] boot sweep failed");
            0
        }
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
