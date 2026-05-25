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
    ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};

/// One provider-backed connection that the memory sync layer can execute.
#[derive(Debug, Clone)]
pub struct SyncTarget {
    pub toolkit: String,
    pub connection_id: String,
}

/// List active Composio connections that have a native memory-sync provider.
pub async fn list_sync_targets(config: &Config) -> Result<Vec<SyncTarget>, String> {
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
pub async fn run_connection_sync(
    config: Config,
    connection_id: &str,
    reason: SyncReason,
) -> Result<SyncOutcome, String> {
    init_default_composio_sync_providers();

    let target = list_sync_targets(&config)
        .await?
        .into_iter()
        .find(|target| target.connection_id == connection_id)
        .ok_or_else(|| {
            format!("no provider-backed active sync target for connection_id={connection_id}")
        })?;

    let provider = get_composio_sync_provider(&target.toolkit).ok_or_else(|| {
        format!(
            "no native memory sync provider registered for toolkit '{}'",
            target.toolkit
        )
    })?;

    let ctx = ProviderContext {
        config: std::sync::Arc::new(config),
        toolkit: target.toolkit,
        connection_id: Some(target.connection_id),
    };

    provider.sync(&ctx, reason).await
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
