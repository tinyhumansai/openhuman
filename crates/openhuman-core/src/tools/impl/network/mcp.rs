use crate::mcp::config_servers::{McpDefinitionAuth, McpRegistrySource, McpServerRegistry};
use crate::security::{SecurityPolicy, ToolOperation};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct McpListServersTool {
    registry: Arc<McpServerRegistry>,
}

impl McpListServersTool {
    pub fn new(registry: Arc<McpServerRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for McpListServersTool {
    fn name(&self) -> &str {
        "mcp_list_servers"
    }

    fn description(&self) -> &str {
        "List named remote MCP servers registered in OpenHuman core. Use this before browsing tools on a specific MCP server."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let servers = self
            .registry
            .list()
            .into_iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "endpoint": endpoint_without_query(&server.endpoint),
                    "description": server.description,
                    "timeout_secs": server.timeout_secs,
                    "allowed_tools": server.allowed_tools,
                    "disallowed_tools": server.disallowed_tools,
                    "auth_configured": !matches!(server.auth, McpDefinitionAuth::None),
                    "auth_kind": auth_kind(&server.auth),
                    "source": server.source,
                })
            })
            .collect::<Vec<_>>();

        let markdown = if servers.is_empty() {
            "# MCP Servers\n\nNo remote MCP servers are registered.".to_string()
        } else {
            let mut md = String::from("# MCP Servers\n");
            for server in self.registry.list() {
                let source = match server.source {
                    McpRegistrySource::Config => "config",
                    McpRegistrySource::Host => "host",
                    _ => "unknown",
                };
                md.push_str(&format!(
                    "\n- **{}** ({source})\n  - endpoint: `{}`\n  - auth: `{}`",
                    server.name,
                    endpoint_without_query(&server.endpoint),
                    auth_kind(&server.auth)
                ));
                if let Some(description) = server.description.as_deref() {
                    md.push_str(&format!("\n  - {description}"));
                }
                if !server.allowed_tools.is_empty() {
                    md.push_str(&format!(
                        "\n  - allowed tools: `{}`",
                        server.allowed_tools.join("`, `")
                    ));
                }
                if !server.disallowed_tools.is_empty() {
                    md.push_str(&format!(
                        "\n  - disallowed tools: `{}`",
                        server.disallowed_tools.join("`, `")
                    ));
                }
            }
            md
        };

        Ok(ToolResult::success_with_markdown(
            json!({ "servers": servers }),
            markdown,
        ))
    }
}

pub struct McpListToolsTool {
    registry: Arc<McpServerRegistry>,
}

impl McpListToolsTool {
    pub fn new(registry: Arc<McpServerRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for McpListToolsTool {
    fn name(&self) -> &str {
        "mcp_list_tools"
    }

    fn description(&self) -> &str {
        "List tools exposed by a named remote MCP server. Use this before calling `mcp_call_tool`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Registered MCP server name from `mcp_list_servers`."
                }
            },
            "required": ["server"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let server = required_string_arg(&args, "server")?;
        let tools = match self.registry.list_tools(&server).await {
            Ok(tools) => tools,
            Err(err) => return Ok(ToolResult::error(format!("mcp_list_tools failed: {err}"))),
        };

        let payload = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "title": tool.display_title(),
                    "description": tool.display_description(),
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();

        let mut markdown = format!("# MCP Tools: `{server}`\n");
        if tools.is_empty() {
            markdown.push_str("\nNo tools were returned by the remote server.");
        } else {
            for tool in &tools {
                let desc = tool
                    .display_description()
                    .unwrap_or_else(|| "No description.".to_string());
                markdown.push_str(&format!(
                    "\n- **{}**: {}\n  - schema: `{}`",
                    tool.name,
                    desc,
                    serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".into())
                ));
            }
        }

        Ok(ToolResult::success_with_markdown(
            json!({ "server": server, "tools": payload }),
            markdown,
        ))
    }
}

pub struct McpCallTool {
    registry: Arc<McpServerRegistry>,
    security: Arc<SecurityPolicy>,
}

impl McpCallTool {
    pub fn new(registry: Arc<McpServerRegistry>, security: Arc<SecurityPolicy>) -> Self {
        Self { registry, security }
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn name(&self) -> &str {
        "mcp_call_tool"
    }

    fn description(&self) -> &str {
        "Call a tool on a named remote MCP server. First inspect available tools with `mcp_list_tools`, then pass the remote tool name and its JSON arguments here."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Registered MCP server name from `mcp_list_servers`."
                },
                "tool": {
                    "type": "string",
                    "description": "Remote MCP tool name from `mcp_list_tools`."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments object passed through to the remote MCP tool."
                }
            },
            "required": ["server", "tool", "arguments"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(|err| anyhow::anyhow!(err))?;

        let server = required_string_arg(&args, "server")?;
        let tool = required_string_arg(&args, "tool")?;
        let arguments = args
            .get("arguments")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required `arguments` object"))?;
        if !arguments.is_object() {
            return Ok(ToolResult::error("`arguments` must be an object"));
        }

        let mut result = match self.registry.call_tool(&server, &tool, arguments).await {
            Ok(result) => result.rendered,
            Err(err) => return Ok(ToolResult::error(format!("mcp_call_tool failed: {err}"))),
        };

        if options.prefer_markdown && result.markdown_formatted.is_none() {
            result.markdown_formatted = Some(result.output());
        }
        Ok(crate::skills::types::tool_result_from_mcp(result))
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }
}

fn auth_kind(auth: &McpDefinitionAuth) -> &'static str {
    match auth {
        McpDefinitionAuth::None => "none",
        McpDefinitionAuth::BearerToken { .. } => "bearer_token",
        McpDefinitionAuth::Basic { .. } => "basic",
        McpDefinitionAuth::Header { .. } => "header",
        McpDefinitionAuth::Headers { .. } => "headers",
        McpDefinitionAuth::QueryParam { .. } => "query_param",
        _ => "unknown",
    }
}

fn endpoint_without_query(endpoint: &str) -> &str {
    endpoint
        .find(['?', '#'])
        .map_or(endpoint, |cut| &endpoint[..cut])
}

fn required_string_arg(args: &Value, key: &str) -> anyhow::Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required `{key}`"))?;
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
