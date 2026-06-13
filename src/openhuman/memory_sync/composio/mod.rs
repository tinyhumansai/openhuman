//! Composio-backed sync pipelines.
//!
//! This module owns the "pull upstream provider data into memory" side of
//! Composio integrations:
//!
//! - provider sync implementations (`providers/*/provider.rs`, `sync.rs`)
//! - periodic scheduler (`periodic.rs`)
//! - trigger / connection-created event subscribers (`bus.rs`)
//! - sync-state persistence and profile-to-memory shaping
//!
//! The sibling [`crate::openhuman::composio`] domain still owns auth,
//! connection management, action execution, and general Composio RPC/tool
//! surfaces. This submodule is specifically the memory-sync half of that
//! integration boundary.

pub mod bus;
pub mod periodic;
pub mod providers;

use crate::openhuman::composio::client::{
    create_composio_client, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::types::ComposioConnection;
use crate::openhuman::config::Config;

pub use bus::{
    register_composio_trigger_subscriber, ComposioConfigChangedSubscriber,
    ComposioTriggerSubscriber,
};
pub use periodic::{record_sync_success, start_periodic_sync};
pub use providers::{
    all_providers as all_composio_sync_providers, get_provider as get_composio_sync_provider,
    init_default_providers as init_default_composio_sync_providers, ComposioProvider,
    ComposioUsage, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};

/// One provider-backed connection that the memory sync layer can execute.
#[derive(Debug, Clone)]
pub struct SyncTarget {
    pub toolkit: String,
    pub connection_id: String,
}

/// List active Composio connections that have a native memory-sync provider.
///
/// When memory_sources entries exist with `kind=composio` and `enabled=true`,
/// those are used as the authoritative source list (user curated). When no
/// memory_sources composio entries exist, falls back to scanning all active
/// Composio connections (legacy behavior).
pub async fn list_sync_targets(config: &Config) -> Result<Vec<SyncTarget>, String> {
    init_default_composio_sync_providers();

    // Try memory_sources registry first (user-curated list).
    let registry_sources = crate::openhuman::memory_sources::list_enabled_by_kind(
        crate::openhuman::memory_sources::SourceKind::Composio,
    )
    .await
    .unwrap_or_default();

    if !registry_sources.is_empty() {
        let from_registry: Vec<SyncTarget> = registry_sources
            .into_iter()
            .filter_map(|s| {
                let toolkit = s.toolkit?;
                let connection_id = s.connection_id?;
                get_composio_sync_provider(&toolkit).map(|_| SyncTarget {
                    toolkit,
                    connection_id,
                })
            })
            .collect();
        if !from_registry.is_empty() {
            tracing::debug!(
                count = from_registry.len(),
                "[composio:sync] using memory_sources registry for sync targets"
            );
            return Ok(from_registry);
        }
        // Registry has entries but none yielded a valid target (missing
        // fields or unregistered toolkit). Fall through to a fresh scan
        // rather than reporting an empty target list — otherwise newly
        // connected integrations stay invisible until reconcile runs.
        tracing::debug!(
            "[composio:sync] registry yielded zero valid targets; falling back to connection scan"
        );
    } else {
        tracing::debug!(
            "[composio:sync] no memory_sources entries; falling back to connection scan"
        );
    }

    scan_active_sync_targets(config).await
}

/// Scan all active Composio connections that have a native memory-sync
/// provider. Always hits Composio directly — does not consult the
/// memory_sources registry. Used by reconciliation to seed the registry.
pub async fn scan_active_sync_targets(config: &Config) -> Result<Vec<SyncTarget>, String> {
    init_default_composio_sync_providers();

    let kind =
        create_composio_client(config).map_err(|e| format!("create_composio_client: {e:#}"))?;
    let response = match kind {
        ComposioClientKind::Backend(client) => client
            .list_connections()
            .await
            .map_err(|e| format!("list_connections (backend): {e:#}"))?,
        ComposioClientKind::Direct(client) => direct_list_connections(&client)
            .await
            .map_err(|e| format!("list_connections (direct): {e:#}"))?,
    };

    Ok(response
        .connections
        .into_iter()
        .filter_map(connection_to_sync_target)
        .collect())
}

/// Run one provider-backed sync end-to-end in-process.
///
/// Returns the provider's [`SyncOutcome`] together with the
/// [`ComposioUsage`] tally (billable action count + actual USD cost)
/// accumulated at the `execute` chokepoint during this run, so the
/// sync-audit caller can record Composio API-call cost alongside the LLM
/// summarisation cost (#3111).
pub async fn run_connection_sync(
    config: Config,
    connection_id: &str,
    reason: SyncReason,
) -> Result<(SyncOutcome, ComposioUsage), (String, ComposioUsage)> {
    init_default_composio_sync_providers();

    let no_usage = |e: String| (e, ComposioUsage::default());

    let target = list_sync_targets(&config)
        .await
        .map_err(no_usage)?
        .into_iter()
        .find(|target| target.connection_id == connection_id)
        .ok_or_else(|| {
            no_usage(format!(
                "no provider-backed active sync target for connection_id={connection_id}",
            ))
        })?;

    let provider = get_composio_sync_provider(&target.toolkit).ok_or_else(|| {
        no_usage(format!(
            "no native memory sync provider registered for toolkit '{}'",
            target.toolkit,
        ))
    })?;

    // Look up the source entry to obtain any user-configured caps.
    // Non-fatal: if the registry read fails we proceed uncapped.
    let (src_max_items, src_sync_depth_days) = {
        let registry_sources = crate::openhuman::memory_sources::list_enabled_by_kind(
            crate::openhuman::memory_sources::SourceKind::Composio,
        )
        .await
        .unwrap_or_default();
        registry_sources
            .iter()
            .find(|s| s.connection_id.as_deref() == Some(&target.connection_id))
            .map(|s| (s.max_items, s.sync_depth_days))
            .unwrap_or((None, None))
    };

    tracing::debug!(
        connection_id = %target.connection_id,
        max_items = ?src_max_items,
        sync_depth_days = ?src_sync_depth_days,
        "[composio:sync] run_connection_sync: caps from registry"
    );

    let ctx = ProviderContext {
        config: std::sync::Arc::new(config),
        toolkit: target.toolkit,
        connection_id: Some(target.connection_id),
        usage: Default::default(),
        max_items: src_max_items,
        sync_depth_days: src_sync_depth_days,
    };

    let sync_result = provider.sync(&ctx, reason).await;

    // Read the Composio billable-action tally *before* propagating errors.
    // A sync that errors partway may still have fired billable actions;
    // reading here ensures the dispatcher audit sees partial cost (#3111).
    let usage = ctx
        .usage
        .lock()
        .map(|u| u.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone());

    match sync_result {
        Ok(outcome) => Ok((outcome, usage)),
        Err(e) => Err((e, usage)),
    }
}

fn connection_to_sync_target(connection: ComposioConnection) -> Option<SyncTarget> {
    if !connection.is_active() {
        return None;
    }
    let toolkit = connection.normalized_toolkit();
    get_composio_sync_provider(&toolkit).map(|_| SyncTarget {
        toolkit,
        connection_id: connection.id,
    })
}
