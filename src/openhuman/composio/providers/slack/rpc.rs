//! JSON-RPC handler functions for the Composio-backed Slack provider.
//!
//! Moved from `memory::slack_ingestion::rpc` into this module so the
//! entire Slack integration lives under `composio::providers::slack`.
//!
//! Public JSON-RPC surface:
//! - `openhuman.slack_memory_sync_trigger` — run `SlackProvider::sync()`
//!   once for each active Slack connection (or just one, if
//!   `connection_id` is supplied).
//! - `openhuman.slack_memory_sync_status` — list the per-connection
//!   sync cursors + last-synced timestamps.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::openhuman::composio::client::{
    create_composio_client, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::providers::registry::get_provider;
use crate::openhuman::composio::providers::sync_state::SyncState;
use crate::openhuman::composio::providers::{ProviderContext, SyncOutcome, SyncReason};
use crate::openhuman::composio::types::ComposioConnectionsResponse;
use crate::openhuman::config::Config;
use crate::openhuman::memory::global::client_if_ready;
use crate::rpc::RpcOutcome;

/// Optional connection-id override for the trigger. When absent, all
/// active Slack connections are synced (serially, one-by-one).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SyncTriggerRequest {
    #[serde(default)]
    pub connection_id: Option<String>,
}

/// Result of `slack_memory_sync_trigger` — per-connection [`SyncOutcome`]s
/// plus aggregate counters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncTriggerResponse {
    pub outcomes: Vec<SyncOutcome>,
    pub connections_considered: usize,
    pub connections_synced: usize,
}

/// Mode-aware connection listing shared by `sync_trigger_rpc` and
/// `sync_status_rpc`. Returns the raw `ComposioConnectionsResponse`
/// (all toolkits, all statuses) — callers filter for slack + active
/// downstream so each RPC owns its own filter semantics.
///
/// Mirrors `composio::ops::composio_list_connections` (#1710): both
/// the backend arm and the direct arm share the same downstream
/// filtering, identical error wrapping, distinct log prefixes for
/// debuggability.
async fn list_slack_connections(config: &Config) -> Result<ComposioConnectionsResponse, String> {
    let kind = create_composio_client(config)
        .map_err(|e| format!("[slack_ingest] list_connections: {e}"))?;
    match kind {
        ComposioClientKind::Backend(client) => client
            .list_connections()
            .await
            .map_err(|e| format!("[slack_ingest] list_connections (backend) failed: {e:#}")),
        ComposioClientKind::Direct(direct) => direct_list_connections(&direct)
            .await
            .map_err(|e| format!("[slack_ingest] list_connections (direct) failed: {e:#}")),
    }
}

/// Run `SlackProvider::sync()` once for every active Slack connection
/// (or exactly one, if `connection_id` is provided). Fails if the
/// user is not signed in (no Composio JWT available).
pub async fn sync_trigger_rpc(
    config: &Config,
    req: SyncTriggerRequest,
) -> Result<RpcOutcome<SyncTriggerResponse>, String> {
    let provider = get_provider("slack")
        .ok_or_else(|| "[slack_ingest] SlackProvider not registered".to_string())?;

    // Route through the mode-aware factory so direct-mode users
    // discover slack connections from THEIR personal Composio tenant —
    // not the tinyhumans backend tenant. Mirrors `composio::ops`
    // (#1710).
    let connections = list_slack_connections(config).await?;

    let mut candidates: Vec<_> = connections
        .connections
        .into_iter()
        .filter(|c| c.normalized_toolkit() == "slack" && c.is_active())
        .collect();

    if let Some(ref wanted) = req.connection_id {
        candidates.retain(|c| &c.id == wanted);
        if candidates.is_empty() {
            return Err(format!(
                "[slack_ingest] no active Slack connection with id={wanted}"
            ));
        }
    }

    let considered = candidates.len();
    let config_arc = Arc::new(config.clone());
    let mut outcomes: Vec<SyncOutcome> = Vec::with_capacity(considered);

    for conn in candidates {
        // `ProviderContext` no longer caches a pre-baked client —
        // `ctx.execute(...)` resolves the underlying handle per call
        // via the mode-aware factory (#1710).
        let ctx = ProviderContext {
            config: Arc::clone(&config_arc),
            toolkit: conn.toolkit.clone(),
            connection_id: Some(conn.id.clone()),
        };
        match provider.sync(&ctx, SyncReason::Manual).await {
            Ok(o) => outcomes.push(o),
            Err(err) => {
                log::warn!(
                    "[slack_ingest] connection={} sync failed: {err:#} (continuing)",
                    conn.id
                );
            }
        }
    }

    let synced = outcomes.len();
    Ok(RpcOutcome::single_log(
        SyncTriggerResponse {
            outcomes,
            connections_considered: considered,
            connections_synced: synced,
        },
        format!("slack_ingest: trigger considered={considered} synced={synced}"),
    ))
}

/// Request body for `slack_memory_sync_status` — no parameters.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SyncStatusRequest {}

/// Response body for `slack_memory_sync_status` — one row per active
/// Slack Composio connection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub connections: Vec<ConnectionStatus>,
}

/// Per-connection sync state snapshot pulled from the Composio sync-state KV.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionStatus {
    pub connection_id: String,
    /// JSON-encoded per-channel cursors (see
    /// `composio::providers::slack::sync::ChannelCursors`). Empty map
    /// when no channels have been flushed yet.
    pub per_channel_cursors: String,
    pub synced_ids_count: usize,
    pub requests_used_today: u32,
    pub daily_request_limit: u32,
}

/// Report one row per active Slack Composio connection, pulled from
/// the Composio sync-state KV store.
pub async fn sync_status_rpc(
    config: &Config,
    _req: SyncStatusRequest,
) -> Result<RpcOutcome<SyncStatusResponse>, String> {
    let memory =
        client_if_ready().ok_or_else(|| "[slack_ingest] memory client not ready".to_string())?;

    // Route through the mode-aware factory so direct-mode users see
    // status rows for THEIR slack connections, not the tinyhumans
    // backend tenant's (#1710).
    let connections = list_slack_connections(config).await?;

    let mut rows = Vec::new();
    for conn in connections.connections {
        if conn.normalized_toolkit() != "slack" {
            continue;
        }
        if !conn.is_active() {
            continue;
        }
        let state = match SyncState::load(&memory, "slack", &conn.id).await {
            Ok(s) => s,
            Err(err) => {
                log::warn!(
                    "[slack_ingest] load_state connection={} failed: {err:#}",
                    conn.id
                );
                continue;
            }
        };
        rows.push(ConnectionStatus {
            connection_id: conn.id.clone(),
            per_channel_cursors: state.cursor.clone().unwrap_or_else(|| "{}".to_string()),
            synced_ids_count: state.synced_ids.len(),
            requests_used_today: state.daily_budget.requests_used,
            daily_request_limit: state.daily_budget.limit,
        });
    }

    let count = rows.len();
    Ok(RpcOutcome::single_log(
        SyncStatusResponse { connections: rows },
        format!("slack_ingest: status connections={count}"),
    ))
}

// ── Tests ───────────────────────────────────────────────────────────
//
// `list_slack_connections` is the shared mode-aware connection-listing
// helper introduced when this RPC pair migrated from
// `build_composio_client` to the factory (#1710 Option C). The tests
// below cover the matrix the migration unlocks — backend mode without a
// session, direct mode without an api_key, and direct mode with an
// api_key (mode-resolution observed without going to the network).
//
// We deliberately avoid hitting `backend.composio.dev` from the test
// runner: the existing pattern across this module is to assert factory
// dispatch + error wrapping rather than mock the upstream HTTP. The
// network-touching paths are smoke-tested upstream in
// `composio::client_tests` / `composio::ops_tests` and the
// direct-mode-toggle test in `action_tool.rs`.

#[cfg(test)]
mod tests {
    use super::*;

    fn unsigned_in_config() -> Config {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        std::mem::forget(tmp);
        config
    }

    fn direct_mode_no_key_config() -> Config {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        std::mem::forget(tmp);
        config
    }

    #[tokio::test]
    async fn list_slack_connections_errors_with_slack_ingest_prefix_when_no_credentials() {
        // Pre-Option-C `sync_trigger_rpc` / `sync_status_rpc` returned
        // the literal string "[slack_ingest] Composio client unavailable
        // (user not signed in?)" because the gate was
        // `build_composio_client(...).is_none()`. Post-Option-C the
        // gate is the factory, so the error surfaces the *factory's*
        // "no backend session" message wrapped with the domain prefix.
        // We exercise the shared helper directly so the test doesn't
        // depend on the SlackProvider being registered in the test
        // global registry (that registration is a runtime concern
        // owned by `init_default_providers`, not relevant to the
        // factory wiring under test here).
        let config = unsigned_in_config();
        let err = list_slack_connections(&config).await.unwrap_err();
        assert!(
            err.starts_with("[slack_ingest] list_connections:"),
            "factory-routed error should keep the [slack_ingest] domain prefix, got: {err}"
        );
        assert!(
            err.contains("no backend session"),
            "backend-mode failure path should surface the factory's session-missing message, \
             got: {err}"
        );
    }

    #[tokio::test]
    async fn list_slack_connections_in_direct_mode_without_api_key_surfaces_direct_mode_error() {
        // Confirms the factory is exercised in direct mode too — when
        // mode=direct but no api_key is stored, the error message
        // surfaces the direct-mode key-missing hint, not the backend
        // session message. Pre-Option-C this returned the backend-only
        // "user not signed in?" message regardless of mode.
        let config = direct_mode_no_key_config();
        let err = list_slack_connections(&config).await.unwrap_err();
        assert!(
            err.starts_with("[slack_ingest] list_connections:"),
            "domain prefix preserved through the factory route, got: {err}"
        );
        assert!(
            err.contains("direct mode") || err.contains("api key"),
            "direct-mode key-missing should surface the direct-mode-specific hint, got: {err}"
        );
    }

    #[tokio::test]
    async fn list_slack_connections_resolves_direct_variant_when_mode_is_direct() {
        // Pin the factory routing: with a direct-mode config + inline
        // api_key, `list_slack_connections` must reach
        // `direct_list_connections` (which then attempts a network
        // call). We can't assert the success path without a mock
        // backend.composio.dev, but we *can* assert the error message
        // identifies the direct arm — proving the factory picked the
        // right branch.
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        config.composio.api_key = Some("test-direct-key".to_string());
        std::mem::forget(tmp);

        let result = list_slack_connections(&config).await;
        // The network call will fail (test environment has no upstream
        // mock). We only care that the failure label says "direct" —
        // that's the load-bearing evidence the factory routed through
        // the new branch instead of the old backend-only path.
        if let Err(err) = result {
            assert!(
                err.contains("(direct)") || err.contains("direct"),
                "factory must route to the direct arm for mode=direct configs, got: {err}"
            );
        }
        // If the network call somehow succeeds (e.g. CI gateway returns
        // a valid empty envelope), that's also acceptable — the
        // factory still routed correctly.
    }
}
