//! `mcp_list_tools`: the tools of one configured static server, scrubbed,
//! screened and projected for display.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{PermissionLevel, Tool, ToolResult};

use super::{bridge_tool_spec, input_module_error_message, required_string_arg};
use crate::{
    config::Config,
    mcp::{
        host,
        module_client::{McpConfiguredClient, SafeStaticToolsError},
    },
};

/// The host adapter for browsing tools on one configured static server.
pub struct McpListToolsTool {
    config: Arc<Config>,
    spec: tinymcp_bus::agent_tools::AgentToolSpec,
}

impl McpListToolsTool {
    /// Builds the browsing tool for one configured MCP client.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        let spec = bridge_tool_spec("mcp_list_tools");
        Self { config, spec }
    }
}

#[async_trait]
impl Tool for McpListToolsTool {
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

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let server = required_string_arg(&args, "server")?;
        let client = match McpConfiguredClient::for_config(&self.config) {
            Ok(client) => client,
            Err(_) => {
                return Ok(ToolResult::error(
                    "mcp_list_tools failed: TinyMCP is unavailable",
                ));
            }
        };
        let server = client
            .normalize_identifier(&server)
            .await
            .map_err(|error| anyhow::anyhow!(input_module_error_message(error)))?;
        let client_config = host::client_config(&self.config).await;
        // Screened on the raw definitions, as every other static-tool path
        // screens them, so this lists exactly what the direct executors accept.
        let remote = match client.safe_static_tools(&client_config, &server).await {
            Ok(remote) => remote,
            Err(SafeStaticToolsError::List(_)) => {
                return Ok(ToolResult::error(
                    "mcp_list_tools failed: module call failed",
                ));
            }
            Err(SafeStaticToolsError::Scrub(_)) => {
                return Ok(ToolResult::error(
                    "mcp_list_tools failed: metadata scrubbing failed",
                ));
            }
        };

        let mut payload = Vec::with_capacity(remote.len());
        for tool in remote {
            let projected: tinymcp_bus::RemoteToolDisplay = match client
                .call(
                    &client_config,
                    tinymcp_bus::names::methods::DISPLAY_REMOTE_TOOL,
                    (tool.clone(),),
                )
                .await
            {
                Ok(projected) => projected,
                Err(_) => {
                    return Ok(ToolResult::error(
                        "mcp_list_tools failed: tool display failed",
                    ));
                }
            };
            payload.push(json!({
                "name": tool.name,
                "title": projected.title,
                "description": projected.description,
                "input_schema": tool.input_schema,
            }));
        }
        let mut markdown = format!("# MCP Tools: `{server}`\n");
        if payload.is_empty() {
            markdown.push_str("\nNo tools were returned by the remote server.");
        } else {
            for tool in &payload {
                let name = tool["name"].as_str().unwrap_or_default();
                let description = tool["description"]
                    .as_str()
                    .or_else(|| tool["title"].as_str())
                    .unwrap_or("No description.");
                let schema = serde_json::to_string(&tool["input_schema"])
                    .unwrap_or_else(|_| "{}".to_owned());
                markdown.push_str(&format!(
                    "\n- **{name}**: {description}\n  - schema: `{schema}`"
                ));
            }
        }
        Ok(ToolResult::success_with_markdown(
            json!({ "server": server, "tools": payload }),
            markdown,
        ))
    }
}
