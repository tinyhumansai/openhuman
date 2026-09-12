//! Host layer over the Composio memory-sync domain.
//!
//! This file used to be `pub use tinymemory_core::sync::composio::*;` — a
//! glob over the engine's entire in-process Composio pipeline. tinymemory
//! v1.13.4 deleted that pipeline outright (72 files, ~18.3k lines, commits
//! `6007de4`/`cb9221b`/`71f4197`): reaching a connected account now needs a
//! credential the engine must not hold, and every toolkit-keyed entry point
//! on `MemorySourceSync` unconditionally refuses. What replaced it is
//! host-initiated: the host reads through the `tinyconnectors` module and
//! hands the resulting records to the bound driver's
//! `MemorySourceSink::accept_source_items`. See
//! `crate::openhuman::integrations::composio::ops::providers_ops::run_sync_pass`
//! for the shared implementation every sync entry point in this domain now
//! calls through.
//!
//! # What survives here, and why
//!
//! [`SyncTarget`] / [`list_sync_targets`] — this host's own replacement for
//! the engine's target discovery, ported onto `memory::sources` (the
//! user-curated registry) with a live-connection-scan fallback, using
//! [`crate::openhuman::integrations::composio::providers::has_native_provider`]
//! in place of the deleted provider registry's `get_provider(toolkit).is_some()`.
//! `memory::ops::sync`'s manual trigger path is still the one caller.
//!
//! `SyncOutcome` / `SyncReason` / `ProviderUserProfile` are **not**
//! re-exported from here any more — they never depended on the engine to
//! begin with (`tinymemory-bus` defines them) and every consumer already
//! reaches them through `integrations::composio::providers`, which is the one
//! true path now that this module's glob cannot supply them as a side
//! effect.
//!
//! `ComposioProvider`, `ProviderContext`, `ProviderArc` and the provider
//! registry (`all_providers`/`get_provider`/`register_provider`) are gone
//! with no replacement — see
//! `crate::openhuman::integrations::composio::providers` for where each of
//! their former callers now gets its answer.

pub mod providers;

pub mod bus;

pub use bus::{
    register_composio_trigger_subscriber, ComposioConfigChangedSubscriber,
    ComposioTriggerSubscriber,
};

use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::providers::has_native_provider;

/// One provider-backed connection the memory sync layer can execute.
#[derive(Debug, Clone)]
pub struct SyncTarget {
    pub toolkit: String,
    pub connection_id: String,
}

/// List active Composio connections that have a native memory-sync provider.
///
/// When `memory_sources` entries exist with `kind=composio` and
/// `enabled=true`, those are used as the authoritative source list (user
/// curated). When no `memory_sources` composio entries exist, falls back to
/// scanning all active Composio connections.
///
/// Ported from the deleted engine's `sync::composio::list_sync_targets`
/// verbatim in behaviour, substituting `has_native_provider` for the deleted
/// provider registry's `get_provider(toolkit).is_some()` — the two answered
/// the same six toolkits (gmail, notion, slack, clickup, github, linear) by
/// construction, since the catalog's `NATIVE_PROVIDERS` table was always kept
/// in step with the registry it described.
pub async fn list_sync_targets(config: &Config) -> Result<Vec<SyncTarget>, String> {
    let registry_sources = crate::openhuman::memory::sources::registry::list_enabled_by_kind(
        crate::openhuman::memory::sources::SourceKind::Composio,
    )
    .await
    .unwrap_or_default();

    if !registry_sources.is_empty() {
        let from_registry: Vec<SyncTarget> = registry_sources
            .into_iter()
            .filter_map(|s| {
                let toolkit = s.toolkit?;
                let connection_id = s.connection_id?;
                has_native_provider(&toolkit).then_some(SyncTarget {
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
/// `memory_sources` registry. Used by reconciliation to seed the registry.
pub async fn scan_active_sync_targets(config: &Config) -> Result<Vec<SyncTarget>, String> {
    let connections =
        crate::openhuman::integrations::composio::ops::composio_list_connections(config)
            .await
            .map_err(|error| format!("list_connections: {error}"))?
            .value
            .connections;

    Ok(connections
        .into_iter()
        .filter(|connection| connection.is_active())
        .filter(|connection| has_native_provider(&connection.normalized_toolkit()))
        .map(|connection| SyncTarget {
            toolkit: connection.normalized_toolkit(),
            connection_id: connection.id,
        })
        .collect())
}
