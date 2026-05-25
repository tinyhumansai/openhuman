//! Global in-process registry of active MCP client connections.
//!
//! Keyed by `server_id` (UUID). Connections are established by [`connect`]
//! and removed by [`disconnect`]. The actual stdio transport lives in
//! [`crate::openhuman::mcp_client::McpStdioClient`] — this module just
//! owns the per-server lifecycle and a global handle map.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tokio::sync::RwLock;

use crate::openhuman::config::Config;
use crate::openhuman::mcp_client::{McpRemoteTool, McpStdioClient};

use super::store;
use super::types::{ConnStatus, InstalledServer, McpTool, ServerStatus};

// ── Connection record ────────────────────────────────────────────────────────

/// One live MCP subprocess plus the tool list cached after `initialize`.
struct Connection {
    client: Arc<McpStdioClient>,
    tools: RwLock<Vec<McpTool>>,
}

impl Connection {
    async fn tools_snapshot(&self) -> Vec<McpTool> {
        self.tools.read().await.clone()
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static CONNECTIONS: OnceLock<RwLock<HashMap<String, Arc<Connection>>>> = OnceLock::new();

fn connections() -> &'static RwLock<HashMap<String, Arc<Connection>>> {
    CONNECTIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn a new stdio subprocess (via `McpStdioClient`), run `initialize`,
/// cache the tool list, and store the connection in the global registry.
pub async fn connect(config: &Config, server: &InstalledServer) -> anyhow::Result<Vec<McpTool>> {
    tracing::debug!(
        "[mcp-registry] connect server_id={} qualified_name={}",
        server.server_id,
        server.qualified_name
    );

    let env_map = store::load_env_values(config, &server.server_id).unwrap_or_default();
    let env: Vec<(String, String)> = env_map.into_iter().collect();

    tracing::debug!(
        "[mcp-registry] connect server_id={} env_keys={:?}",
        server.server_id,
        env.iter().map(|(k, _)| k).collect::<Vec<_>>()
    );

    let identity = config.mcp_client.client_identity.clone();
    let client = Arc::new(McpStdioClient::new(
        server.command.clone(),
        server.args.clone(),
        env,
        None,
        identity,
    ));

    // Initialize + first tools/list happen here so a misconfigured server
    // fails loudly at `connect` instead of silently at first `call_tool`.
    client.initialize().await?;
    let remote_tools = client.list_tools().await?;
    let tools: Vec<McpTool> = remote_tools.into_iter().map(into_registry_tool).collect();

    let conn = Arc::new(Connection {
        client: Arc::clone(&client),
        tools: RwLock::new(tools.clone()),
    });

    {
        let mut map = connections().write().await;
        map.insert(server.server_id.clone(), conn);
    }

    let _ = store::update_last_connected(config, &server.server_id);

    tracing::debug!(
        "[mcp-registry] connect ok server_id={} tools={}",
        server.server_id,
        tools.len()
    );

    Ok(tools)
}

/// Disconnect and remove from the registry.
pub async fn disconnect(server_id: &str) -> bool {
    tracing::debug!("[mcp-registry] disconnect server_id={server_id}");
    let conn = {
        let mut map = connections().write().await;
        map.remove(server_id)
    };
    if let Some(c) = conn {
        let _ = c.client.close_session().await;
        tracing::debug!("[mcp-registry] disconnected server_id={server_id}");
        true
    } else {
        tracing::debug!("[mcp-registry] disconnect noop server_id={server_id}");
        false
    }
}

/// Invoke `tools/call` on a connected server. The MCP `CallToolResult` is
/// returned as the raw JSON value (matches the prior wire contract used by
/// `tool_call`).
pub async fn call_tool(
    server_id: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let conn = {
        let map = connections().read().await;
        map.get(server_id).cloned()
    }
    .ok_or_else(|| format!("[mcp-registry] server_id={server_id} not connected"))?;

    conn.client
        .call_tool(tool_name, arguments)
        .await
        .map(|r| r.raw_result)
        .map_err(|e| e.to_string())
}

/// Return status summaries for all installed servers.
pub async fn all_status(config: &Config) -> Vec<ConnStatus> {
    let installed = store::list_servers(config).unwrap_or_default();
    let connected_ids: Vec<String> = {
        let map = connections().read().await;
        map.keys().cloned().collect()
    };

    let mut out = Vec::with_capacity(installed.len());
    for s in installed {
        let is_connected = connected_ids.iter().any(|id| id == &s.server_id);
        let tool_count = if is_connected {
            let map = connections().read().await;
            match map.get(&s.server_id) {
                Some(c) => c.tools_snapshot().await.len() as u32,
                None => 0,
            }
        } else {
            0
        };
        out.push(ConnStatus {
            server_id: s.server_id,
            qualified_name: s.qualified_name,
            display_name: s.display_name,
            status: if is_connected {
                ServerStatus::Connected
            } else {
                ServerStatus::Disconnected
            },
            tool_count,
            last_error: None,
        });
    }
    out
}

/// Collect tools from all currently-connected servers for tool_registry integration.
/// Returns `(server_id, qualified_name, tool)` triples. `qualified_name` is
/// best-effort sourced from the connection's `server_id` here — callers that
/// need the real qualified name should re-join against `store::list_servers`.
pub async fn all_connected_tools() -> Vec<(String, String, McpTool)> {
    let snapshot: Vec<(String, Arc<Connection>)> = {
        let map = connections().read().await;
        map.iter()
            .map(|(id, c)| (id.clone(), Arc::clone(c)))
            .collect()
    };

    let mut out: Vec<(String, String, McpTool)> = Vec::new();
    for (server_id, c) in snapshot {
        for tool in c.tools_snapshot().await {
            out.push((server_id.clone(), server_id.clone(), tool));
        }
    }
    out
}

// ── Boundary conversion ──────────────────────────────────────────────────────

fn into_registry_tool(remote: McpRemoteTool) -> McpTool {
    McpTool {
        name: remote.name,
        description: remote.description,
        input_schema: remote.input_schema,
    }
}

#[cfg(test)]
mod tests {
    // Live-connection tests require a real MCP subprocess and live in
    // tests/json_rpc_e2e.rs. Keep this slot for sync helper tests.

    #[test]
    fn placeholder_so_module_compiles_under_test_cfg() {
        // Intentionally empty.
    }
}
