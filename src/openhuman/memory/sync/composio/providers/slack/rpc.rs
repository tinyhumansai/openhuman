//! JSON-RPC handler functions for the Composio-backed Slack provider.
//!
//! Moved from `memory::slack_ingestion::rpc` into this module so the
//! entire Slack integration lives under `composio::providers::slack`.
//!
//! Public JSON-RPC surface:
//! - `openhuman.slack_memory_sync_trigger` — read each active Slack
//!   connection through the `tinyconnectors` module and ingest what it
//!   returns (or just one connection, if `connection_id` is supplied).
//! - `openhuman.slack_memory_sync_status` — list the connections a trigger
//!   would act on.
//!
//! # Where the sync actually happens now
//!
//! tinymemory v1.13.4 deleted the in-process `SlackProvider` along with the
//! rest of the Composio pipeline: `MemorySourceSync::run_connection_sync` and
//! `::source_sync_state` now unconditionally refuse for every toolkit,
//! because reaching a connected account needs a credential the engine must
//! not hold. `sync_trigger_rpc` below reads through the connector module and
//! writes into the bound memory driver via `MemorySourceSink::accept_source_items`
//! instead — the same `run_sync_pass` helper
//! `integrations::composio::ops::composio_sync` uses (through
//! `run_sync_within_budget`, which repeats it within one call's item budget),
//! called synchronously here rather than fired into a background task,
//! because this RPC's contract is "return the outcome", not "return that a
//! run started".
//!
//! `sync_status_rpc` genuinely lost capability it cannot honestly recover:
//! the module keeps its sync cursor and daily-request budget internally and
//! exposes neither outside of a `Sync` call, so the per-connection detail
//! `ConnectionStatus` used to report (cursor JSON, synced-id count, requests
//! used today) has no source any more. The rows below still report which
//! connections a trigger would act on, with the detail fields at their zero
//! value rather than removed from the wire shape.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::client::{
    create_composio_client, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::integrations::composio::ops::run_sync_within_budget;
use crate::openhuman::integrations::composio::providers::SyncOutcome;
use crate::openhuman::integrations::composio::types::ComposioConnectionsResponse;
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
    let mut outcomes: Vec<SyncOutcome> = Vec::with_capacity(considered);

    for conn in candidates {
        let started_at_ms = now_ms();
        // Reads through the `tinyconnectors` module and ingests through the
        // bound driver's `MemorySourceSink` — see the module doc comment for
        // why this no longer goes through `MemorySourceSync::run_connection_sync`.
        match run_sync_within_budget(config, "slack", &conn.id, "manual")
            .await
            .map_err(|error| error.to_string())
        {
            Ok(pass) => outcomes.push(SyncOutcome {
                toolkit: "slack".to_string(),
                connection_id: Some(conn.id.clone()),
                reason: "manual".to_string(),
                items_ingested: pass.records_read,
                started_at_ms,
                finished_at_ms: now_ms(),
                summary: format!(
                    "Slack sync completed ({} written, {} already ingested)",
                    pass.written, pass.already_ingested
                ),
                details: serde_json::json!({
                    "more_pending": pass.more_pending,
                    "written": pass.written,
                    "already_ingested": pass.already_ingested,
                }),
            }),
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

/// List every active Slack Composio connection a trigger would act on.
///
/// **Degraded read, and honestly so.** This used to pull the per-connection
/// cursor/dedup/budget snapshot the engine's `SlackProvider` kept in the
/// sync-state KV, through `MemorySourceSync::source_sync_state`. That member
/// now unconditionally refuses for every toolkit (tinymemory v1.13.4 deleted
/// the in-process pipeline it read), and the `tinyconnectors` module keeps its
/// cursor and daily-request budget internally — neither crosses the bus
/// outside of an actual `Sync` call, so there is nothing left to poll for a
/// passive status read. Rather than dropping the fields (a wire-shape change
/// every existing caller of `slack_memory_sync_status` would have to handle),
/// this reports the connections a trigger would consider with the
/// once-populated detail fields at their zero value.
pub async fn sync_status_rpc(
    config: &Config,
    _req: SyncStatusRequest,
) -> Result<RpcOutcome<SyncStatusResponse>, String> {
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
        rows.push(ConnectionStatus {
            connection_id: conn.id.clone(),
            per_channel_cursors: "{}".to_string(),
            synced_ids_count: 0,
            requests_used_today: 0,
            daily_request_limit: 0,
        });
    }

    let count = rows.len();
    Ok(RpcOutcome::single_log(
        SyncStatusResponse { connections: rows },
        format!(
            "slack_ingest: status connections={count} (per-connection sync detail is no \
             longer available — the connector module keeps its cursor internally)"
        ),
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
#[path = "rpc_tests.rs"]
mod tests;
