//! JSON-RPC handler functions for the Composio-backed Slack provider.
//!
//! Public JSON-RPC surface:
//! - `openhuman.slack_memory_sync_trigger` — run `SlackProvider::sync()`
//!   once for each active Slack connection (or just one, if
//!   `connection_id` is supplied).
//! - `openhuman.slack_memory_sync_status` — list the per-connection
//!   sync cursors + last-synced timestamps.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::openhuman::composio::client::build_composio_client;
use crate::openhuman::composio::providers::registry::get_provider;
use crate::openhuman::composio::providers::sync_state::SyncState;
use crate::openhuman::composio::providers::{ProviderContext, SyncOutcome, SyncReason};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncTriggerResponse {
    pub outcomes: Vec<SyncOutcome>,
    pub connections_considered: usize,
    pub connections_synced: usize,
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

    let client = build_composio_client(config).ok_or_else(|| {
        "[slack_ingest] Composio client unavailable (user not signed in?)".to_string()
    })?;

    // Discover connections via the backend; filter for slack ones.
    let connections = client
        .list_connections()
        .await
        .map_err(|e| format!("[slack_ingest] list_connections failed: {e:#}"))?;

    let mut candidates: Vec<_> = connections
        .connections
        .into_iter()
        .filter(|c| {
            c.toolkit.eq_ignore_ascii_case("slack")
                && matches!(c.status.as_str(), "ACTIVE" | "CONNECTED")
        })
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
        let ctx = ProviderContext {
            config: Arc::clone(&config_arc),
            client: client.clone(),
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SyncStatusRequest {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub connections: Vec<ConnectionStatus>,
}

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
    let client = build_composio_client(config).ok_or_else(|| {
        "[slack_ingest] Composio client unavailable (user not signed in?)".to_string()
    })?;
    let memory =
        client_if_ready().ok_or_else(|| "[slack_ingest] memory client not ready".to_string())?;

    let connections = client
        .list_connections()
        .await
        .map_err(|e| format!("[slack_ingest] list_connections failed: {e:#}"))?;

    let mut rows = Vec::new();
    for conn in connections.connections {
        if !conn.toolkit.eq_ignore_ascii_case("slack") {
            continue;
        }
        if !matches!(conn.status.as_str(), "ACTIVE" | "CONNECTED") {
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
