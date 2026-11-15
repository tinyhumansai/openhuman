//! `mcp_list_servers`: the configured static servers, through TinyMCP's safe
//! summary operation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{PermissionLevel, Tool, ToolResult};

use super::bridge_tool_spec;
use crate::{
    config::Config,
    mcp::{host, module_client::McpConfiguredClient},
};

/// Lists configured MCP servers through TinyMCP's safe summary operation.
pub struct McpListServersTool {
    config: Arc<Config>,
    spec: tinymcp_bus::agent_tools::AgentToolSpec,
}

impl McpListServersTool {
    /// Builds the server-list tool for one configured workspace.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            spec: bridge_tool_spec("mcp_list_servers"),
        }
    }
}

#[async_trait]
impl Tool for McpListServersTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters_schema(&self) -> Value {
        self.spec.parameters.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let client = match McpConfiguredClient::for_config(&self.config) {
            Ok(client) => client,
            Err(_) => {
                return Ok(ToolResult::error(
                    "mcp_list_servers failed: TinyMCP is unavailable",
                ));
            }
        };
        let config = host::client_config(&self.config).await;
        let summaries: Vec<tinymcp_bus::StaticServerSummary> = match client
            .call_confidential(
                &config,
                tinymcp_bus::names::methods::STATIC_SERVER_SUMMARIES,
                (),
            )
            .await
        {
            Ok(summaries) => summaries,
            Err(_) => {
                return Ok(ToolResult::error(
                    "mcp_list_servers failed: module call failed",
                ));
            }
        };
        let servers = summaries
            .iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "endpoint": server.endpoint,
                    "description": server.description,
                    "timeout_secs": server.timeout_secs,
                    "allowed_tools": server.allowed_tools,
                    "disallowed_tools": server.disallowed_tools,
                    "auth_configured": server.auth_configured,
                    "auth_kind": server.auth_kind,
                    "source": server.source,
                })
            })
            .collect::<Vec<_>>();
        let mut markdown = String::from("# MCP Servers\n");
        if summaries.is_empty() {
            markdown.push_str("\nNo remote MCP servers are registered.");
        } else {
            for server in &summaries {
                markdown.push_str(&format!(
                    "\n- **{}** ({})\n  - endpoint: `{}`\n  - auth: `{}`",
                    server.name, server.source, server.endpoint, server.auth_kind
                ));
                if let Some(description) = server.description.as_deref() {
                    markdown.push_str(&format!("\n  - {description}"));
                }
                if !server.allowed_tools.is_empty() {
                    markdown.push_str(&format!(
                        "\n  - allowed tools: `{}`",
                        server.allowed_tools.join("`, `")
                    ));
                }
                if !server.disallowed_tools.is_empty() {
                    markdown.push_str(&format!(
                        "\n  - disallowed tools: `{}`",
                        server.disallowed_tools.join("`, `")
                    ));
                }
            }
        }
        Ok(ToolResult::success_with_markdown(
            json!({ "servers": servers }),
            markdown,
        ))
    }
}
