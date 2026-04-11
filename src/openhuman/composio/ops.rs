//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Result alias used by every `composio_*` op in this module.
///
/// We deliberately return a plain `String` error instead of
/// `anyhow::Error` — the controller layer in `schemas.rs` forwards
/// these straight into the RPC envelope, and `String` keeps the shape
/// obvious at a glance.
type OpResult<T> = std::result::Result<T, String>;

use super::client::{build_composio_client, ComposioClient};
use super::types::{
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioDeleteResponse,
    ComposioExecuteResponse, ComposioToolkitsResponse, ComposioToolsResponse,
};

/// Resolve a [`ComposioClient`] from `config.integrations`, or return an
/// error string that the caller can surface over RPC.
fn resolve_client(config: &Config) -> OpResult<ComposioClient> {
    build_composio_client(&config.integrations).ok_or_else(|| {
        "composio is disabled (integrations.enabled or integrations.composio.enabled is off, \
         or backend_url/auth_token missing)"
            .to_string()
    })
}

// ── Toolkits ────────────────────────────────────────────────────────

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");
    let client = resolve_client(config)?;
    let resp = client
        .list_toolkits()
        .await
        .map_err(|e| format!("[composio] list_toolkits failed: {e}"))?;
    let count = resp.toolkits.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} toolkit(s) enabled")],
    ))
}

// ── Connections ─────────────────────────────────────────────────────

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    let client = resolve_client(config)?;
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("[composio] list_connections failed: {e}"))?;
    let active = resp
        .connections
        .iter()
        .filter(|c| matches!(c.status.as_str(), "ACTIVE" | "CONNECTED"))
        .count();
    let total = resp.connections.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {total} connection(s) listed ({active} active)"
        )],
    ))
}

pub async fn composio_authorize(
    config: &Config,
    toolkit: &str,
) -> OpResult<RpcOutcome<ComposioAuthorizeResponse>> {
    tracing::debug!(toolkit = %toolkit, "[composio] rpc authorize");
    let client = resolve_client(config)?;
    let resp = client
        .authorize(toolkit)
        .await
        .map_err(|e| format!("[composio] authorize failed: {e}"))?;

    // Publish an event so any interested subscribers (e.g. UI refreshers,
    // analytics) can react to the new connection handoff.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionCreated {
            toolkit: toolkit.to_string(),
            connection_id: resp.connection_id.clone(),
            connect_url: resp.connect_url.clone(),
        },
    );

    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: authorize flow started for {toolkit}")],
    ))
}

pub async fn composio_delete_connection(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let resp = client
        .delete_connection(connection_id)
        .await
        .map_err(|e| format!("[composio] delete_connection failed: {e}"))?;
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

// ── Tools ───────────────────────────────────────────────────────────

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    tracing::debug!(?toolkits, "[composio] rpc list_tools");
    let client = resolve_client(config)?;
    let resp = client
        .list_tools(toolkits.as_deref())
        .await
        .map_err(|e| format!("[composio] list_tools failed: {e}"))?;
    let count = resp.tools.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} tool(s) listed")],
    ))
}

// ── Execute ─────────────────────────────────────────────────────────

pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, "[composio] rpc execute");
    let client = resolve_client(config)?;
    let started = std::time::Instant::now();
    let result = client.execute_tool(tool, arguments.clone()).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(resp) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: resp.successful,
                    error: resp.error.clone(),
                    cost_usd: resp.cost_usd,
                    elapsed_ms,
                },
            );
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: executed {tool} ({elapsed_ms}ms)")],
            ))
        }
        Err(e) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            Err(format!("[composio] execute failed: {e}"))
        }
    }
}

// ── Helpers re-exported so callers can pull connection/tool types without
// reaching into the nested types module.
pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};
