//! The server operations widget RPCs need, behind a seam.
//!
//! [`HostUiPort`] answers them from this workspace's MCP host: installed
//! servers through the live connection map, configured servers through the
//! static set. Tests substitute their own.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::Config;
use crate::mcp::host;
use crate::mcp::registry::connections;

use super::types::UiToolDescriptor;

/// What a widget RPC asks of an MCP server.
#[async_trait]
pub trait UiServerPort: Send + Sync {
    /// Whether `server_id` is connected (installed) or configured (static).
    async fn is_available(&self, server_id: &str) -> bool;

    /// The live, scan-clean descriptor of `tool`, or `None` when the server
    /// does not offer it now.
    async fn tool_descriptor(&self, server_id: &str, tool: &str) -> Option<UiToolDescriptor>;

    /// A `resources/read` reply's `contents`.
    async fn read_resource(&self, server_id: &str, uri: &str) -> Result<Vec<Value>, String>;

    /// A `tools/call` reply, as the server sent it.
    async fn call_tool(
        &self,
        server_id: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, String>;
}

/// [`UiServerPort`] over `config`'s MCP host.
pub struct HostUiPort {
    config: Arc<Config>,
}

impl HostUiPort {
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    fn is_static(&self, server_id: &str) -> bool {
        host::static_registry(&self.config).get(server_id).is_some()
    }
}

#[async_trait]
impl UiServerPort for HostUiPort {
    async fn is_available(&self, server_id: &str) -> bool {
        connections::is_connected_for_config(&self.config, server_id).await
            || self.is_static(server_id)
    }

    async fn tool_descriptor(&self, server_id: &str, tool: &str) -> Option<UiToolDescriptor> {
        if let Some(live) = connections::server_tools_for_config(&self.config, server_id).await {
            let safe = crate::mcp::registry::tools_safe_for_agent(server_id, live);
            safe.iter().find(|candidate| candidate.name == tool)?;
            let service = host::for_config(&self.config).ok()?;
            let meta = service
                .dynamic()
                .connections()
                .tool_meta(server_id, tool)
                .await;
            return Some(UiToolDescriptor {
                meta,
                annotations: None,
            });
        }
        let registry = host::static_registry(&self.config);
        registry.get(server_id)?;
        let listed = registry
            .list_tools(server_id)
            .await
            .map_err(|error| {
                tracing::debug!(server_id, %error, "[mcp_ui] could not list tools");
            })
            .ok()?;
        let safe = crate::mcp::registry::tools_safe_for_agent(
            server_id,
            listed
                .into_iter()
                .map(|remote| {
                    let description = remote.display_description();
                    tinymcp_bus::McpTool {
                        name: remote.name,
                        description,
                        input_schema: remote.input_schema,
                    }
                })
                .collect(),
        );
        safe.iter().find(|candidate| candidate.name == tool)?;
        Some(UiToolDescriptor {
            meta: registry.tool_meta(server_id, tool),
            annotations: None,
        })
    }

    async fn read_resource(&self, server_id: &str, uri: &str) -> Result<Vec<Value>, String> {
        let contents = if connections::is_connected_for_config(&self.config, server_id).await {
            host::for_config(&self.config)
                .map_err(|error| error.to_string())?
                .dynamic()
                .connections()
                .read_resource(server_id, uri)
                .await
        } else {
            host::static_registry(&self.config)
                .read_resource(server_id, uri)
                .await
        }
        .map_err(|error| {
            tracing::debug!(server_id, %error, "[mcp_ui] resources/read failed");
            error.to_string()
        })?;
        contents
            .iter()
            .map(|entry| serde_json::to_value(entry).map_err(|error| error.to_string()))
            .collect()
    }

    async fn call_tool(
        &self,
        server_id: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        if connections::is_connected_for_config(&self.config, server_id).await {
            let service = host::for_config(&self.config).map_err(|error| error.to_string())?;
            return service
                .dynamic()
                .tool_call(server_id, tool, arguments)
                .await
                .map(|outcome| outcome.result)
                .map_err(|error| error.to_string());
        }
        host::static_registry(&self.config)
            .call_tool(server_id, tool, arguments)
            .await
            .map(|result| result.raw_result)
            .map_err(|error| error.to_string())
    }
}
