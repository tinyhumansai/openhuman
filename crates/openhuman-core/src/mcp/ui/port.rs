//! The server operations widget RPCs need, behind a seam.
//!
//! [`HostUiPort`] answers them through this workspace's TinyMCP module:
//! installed servers by their connection, configured servers by the static
//! set. Widget documents are read through the module's resource members.
//! Tests substitute their own port.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::Config;
use crate::mcp::host;
use crate::mcp::module_client::McpConfiguredClient;

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

/// [`UiServerPort`] over `config`'s TinyMCP module.
pub struct HostUiPort {
    config: Arc<Config>,
    client: Option<Arc<McpConfiguredClient>>,
}

impl HostUiPort {
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            client: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_client(config: Arc<Config>, client: Arc<McpConfiguredClient>) -> Self {
        Self {
            config,
            client: Some(client),
        }
    }

    fn client(&self) -> Option<Arc<McpConfiguredClient>> {
        match &self.client {
            Some(client) => Some(Arc::clone(client)),
            None => McpConfiguredClient::for_config(&self.config).ok(),
        }
    }

    async fn is_static(&self, server_id: &str) -> bool {
        let Some(client) = self.client() else {
            return false;
        };
        let listed: Result<Vec<String>, _> = client
            .call(
                &host::client_config(&self.config),
                tinymcp_bus::names::methods::STATIC_LIST,
                (),
            )
            .await;
        listed.is_ok_and(|servers| servers.iter().any(|name| name == server_id))
    }

    async fn is_installed_connected(&self, server_id: &str) -> bool {
        let Some(client) = self.client() else {
            return false;
        };
        let statuses: Result<Vec<tinymcp_bus::ConnStatus>, _> = client
            .call(
                &host::client_config(&self.config),
                tinymcp_bus::names::methods::STATUS,
                (),
            )
            .await;
        statuses.is_ok_and(|statuses| {
            statuses.iter().any(|status| {
                status.server_id == server_id
                    && status.status == tinymcp_bus::ServerStatus::Connected
            })
        })
    }
}

#[async_trait]
impl UiServerPort for HostUiPort {
    async fn is_available(&self, server_id: &str) -> bool {
        self.is_installed_connected(server_id).await || self.is_static(server_id).await
    }

    async fn tool_descriptor(&self, server_id: &str, tool: &str) -> Option<UiToolDescriptor> {
        let client = self.client()?;
        let client_config = host::client_config(&self.config);
        if self.is_installed_connected(server_id).await {
            let candidate =
                super::decorate::installed_tool(&client, &client_config, server_id, tool).await?;
            return Some(UiToolDescriptor {
                meta: candidate.meta,
                annotations: None,
            });
        }
        let listed: Vec<tinymcp_bus::McpRemoteTool> = client
            .call(
                &client_config,
                tinymcp_bus::names::methods::STATIC_LIST_TOOLS,
                (server_id,),
            )
            .await
            .map_err(|error| {
                tracing::debug!(server_id, ?error, "[mcp_ui] could not list tools");
            })
            .ok()?;
        let listed = client
            .scrub_static_tools(&client_config, server_id, listed)
            .await
            .ok()?;
        let meta = listed
            .iter()
            .find(|remote| remote.name == tool)
            .and_then(|remote| remote.meta.clone());
        let safe = crate::mcp::registry::tools_safe_for_agent(
            server_id,
            listed
                .into_iter()
                .map(|remote| tinymcp_bus::McpTool {
                    description: remote.description.or(remote.title),
                    name: remote.name,
                    input_schema: remote.input_schema,
                    meta: None,
                })
                .collect(),
        );
        safe.iter().find(|candidate| candidate.name == tool)?;
        Some(UiToolDescriptor {
            meta,
            annotations: None,
        })
    }

    async fn read_resource(&self, server_id: &str, uri: &str) -> Result<Vec<Value>, String> {
        let client = self.client().ok_or("MCP module is unavailable")?;
        let member = if self.is_installed_connected(server_id).await {
            tinymcp_bus::names::methods::READ_RESOURCE
        } else {
            tinymcp_bus::names::methods::STATIC_READ_RESOURCE
        };
        let contents: Vec<tinymcp_bus::McpResourceContents> = client
            .call(&host::client_config(&self.config), member, (server_id, uri))
            .await
            .map_err(|error| {
                tracing::debug!(
                    server_id,
                    uri,
                    ?error,
                    "[mcp_ui] widget resource read failed"
                );
                "MCP resource read failed".to_string()
            })?;
        contents
            .iter()
            .map(|content| serde_json::to_value(content).map_err(|error| error.to_string()))
            .collect()
    }

    async fn call_tool(
        &self,
        server_id: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let client = self.client().ok_or("MCP module is unavailable")?;
        let client_config = host::client_config(&self.config);
        let member = if self.is_installed_connected(server_id).await {
            tinymcp_bus::names::methods::TOOL_CALL
        } else {
            tinymcp_bus::names::methods::STATIC_CALL_TOOL
        };
        let outcome: tinymcp_bus::ToolCallOutcome = client
            .call_confidential(&client_config, member, (server_id, tool, arguments))
            .await
            .map_err(|error| {
                tracing::debug!(server_id, tool, ?error, "[mcp_ui] widget tool call failed");
                "MCP tool call failed".to_string()
            })?;
        Ok(outcome.result)
    }
}

#[cfg(test)]
#[path = "port_tests.rs"]
mod tests;
