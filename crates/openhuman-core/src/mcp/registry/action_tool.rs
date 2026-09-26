//! Searchable agent tools for actions on already-connected MCP servers.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolExposure, ToolResult};

use crate::config::Config;
use crate::util::sanitize::sanitize_for_llm;

use super::connections;
use super::types::{ConnectedServerOverview, McpTool};

/// Stable, provider-safe name for one server's tool. The digest distinguishes
/// equal tool names on different servers and names truncated to the same slug.
pub fn searchable_name(server_id: &str, tool_name: &str) -> String {
    let slug: String = tool_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(42)
        .collect();
    let slug = slug.trim_matches('_');
    let slug = if slug.is_empty() { "tool" } else { slug };
    let digest = Sha256::digest(format!("{server_id}\0{tool_name}").as_bytes());
    format!("mcp_{slug}_{}", &hex::encode(digest)[..12])
}

/// One deferred registration per callable action from each connected server.
/// Remote definitions are filtered by the host's injection policy first.
pub fn deferred_connected_tools(
    config: Arc<Config>,
    servers: &[ConnectedServerOverview],
) -> Vec<Box<dyn Tool>> {
    let mut ordered: Vec<&ConnectedServerOverview> = servers.iter().collect();
    ordered.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    let mut names = HashSet::new();
    let mut result: Vec<Box<dyn Tool>> = Vec::new();
    for server in ordered {
        let mut tools = super::tools_safe_for_agent(&server.server_id, server.tools.clone());
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        for tool in tools {
            if tool.name.trim().is_empty() {
                continue;
            }
            let action = McpActionTool::new(Arc::clone(&config), server, tool);
            if names.insert(action.name.clone()) {
                result.push(Box::new(action));
            }
        }
    }
    result
}

struct McpActionTool {
    config: Arc<Config>,
    name: String,
    server_id: String,
    tool_name: String,
    family: String,
    description: String,
    parameters: Value,
}

impl McpActionTool {
    fn new(config: Arc<Config>, server: &ConnectedServerOverview, tool: McpTool) -> Self {
        let name = searchable_name(&server.server_id, &tool.name);
        let server_name = if server.display_name.trim().is_empty() {
            &server.qualified_name
        } else {
            &server.display_name
        };
        let family = sanitize_for_llm(&server.qualified_name, 120);
        let description = format!(
            "MCP server {}: {}",
            sanitize_for_llm(server_name, 120),
            sanitize_for_llm(tool.description.as_deref().unwrap_or(&tool.name), 500)
        );
        let mut parameters = if tool.input_schema.is_object() {
            tool.input_schema
        } else {
            json!({ "type": "object", "properties": {} })
        };
        sanitize_schema_descriptions(&mut parameters);
        Self {
            config,
            name,
            server_id: server.server_id.clone(),
            tool_name: tool.name,
            family,
            description,
            parameters,
        }
    }
}

fn sanitize_schema_descriptions(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if (key == "description" || key == "title") && child.is_string() {
                    *child =
                        Value::String(sanitize_for_llm(child.as_str().unwrap_or_default(), 500));
                } else {
                    sanitize_schema_descriptions(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_schema_descriptions(item);
            }
        }
        _ => {}
    }
}

#[async_trait]
impl Tool for McpActionTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        true
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn family(&self) -> Option<&str> {
        Some(&self.family)
    }

    async fn execute(&self, arguments: Value) -> anyhow::Result<ToolResult> {
        let Some(live_tools) =
            connections::server_tools_for_config(&self.config, &self.server_id).await
        else {
            return Ok(ToolResult::error("MCP server is no longer connected"));
        };
        let safe_tools = super::tools_safe_for_agent(&self.server_id, live_tools);
        if !safe_tools.iter().any(|tool| tool.name == self.tool_name) {
            return Ok(ToolResult::error("MCP tool is no longer available"));
        }
        let outcome = super::ops::mcp_clients_tool_call(
            &self.config,
            self.server_id.clone(),
            self.tool_name.clone(),
            arguments,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let payload = serde_json::to_string(&outcome.value)?;
        if outcome.value.get("is_error").and_then(Value::as_bool) == Some(true) {
            Ok(ToolResult::error(payload))
        } else {
            Ok(ToolResult::success(payload))
        }
    }
}

#[cfg(test)]
#[path = "action_tool_tests.rs"]
mod tests;
