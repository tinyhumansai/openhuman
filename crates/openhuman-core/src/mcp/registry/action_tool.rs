//! Host adapters for the module-prepared tools on installed MCP servers.
//!
//! TinyMCP prepares stable names, descriptions, schemas, exposure and tags.
//! This file applies OpenHuman's prompt-injection policy before preparation
//! and supplies the `tinytools::Tool` adapter that executes through TinyBus.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolContent, ToolExposure, ToolResult};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::mcp::{host, module_client::McpConfiguredClient};

/// Creates deferred host adapters for the module-prepared tools in `tools`.
pub fn deferred_connected_tools(
    config: Arc<Config>,
    tools: &[tinymcp_bus::McpToolPresentation],
) -> Vec<Box<dyn Tool>> {
    deferred_connected_tools_with_legacy(config, tools, &HashSet::new())
}

/// Restores previous tool names recorded in a frozen conversation.
///
/// A resumed session may contain a legacy hashed name. Its tool declaration
/// remains available under that exact name while the current module-prepared
/// name is also exposed.
pub fn deferred_connected_tools_with_legacy(
    config: Arc<Config>,
    tools: &[tinymcp_bus::McpToolPresentation],
    recorded: &HashSet<String>,
) -> Vec<Box<dyn Tool>> {
    let mut result = Vec::with_capacity(tools.len() * 2);
    for presentation in tools {
        let tool = InstalledServerTool {
            config: Arc::clone(&config),
            presentation: presentation.clone(),
            name: presentation.name.clone(),
        };
        if recorded.contains(&presentation.legacy_name)
            && presentation.legacy_name != presentation.name
        {
            result.push(Box::new(InstalledServerTool {
                name: presentation.legacy_name.clone(),
                ..tool.clone()
            }) as Box<dyn Tool>);
        }
        result.push(Box::new(tool));
    }
    result
}

#[derive(Clone)]
struct InstalledServerTool {
    config: Arc<Config>,
    presentation: tinymcp_bus::McpToolPresentation,
    name: String,
}

#[async_trait]
impl Tool for InstalledServerTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.presentation.description
    }

    fn parameters_schema(&self) -> Value {
        self.presentation.parameters.clone()
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
        if self.presentation.deferred {
            ToolExposure::Deferred
        } else {
            ToolExposure::Direct
        }
    }

    fn family(&self) -> Option<&str> {
        Some(&self.presentation.family)
    }

    fn tags(&self) -> Vec<String> {
        self.presentation.tags.clone()
    }

    async fn execute(&self, arguments: Value) -> anyhow::Result<ToolResult> {
        let start = Instant::now();
        let result = self.execute_inner(arguments).await;
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        BUS.publish(DomainEvent::McpClientToolExecuted {
            server_id: self.presentation.server_id.clone(),
            tool_name: self.presentation.remote_name.clone(),
            success: result.as_ref().is_ok_and(|result| !result.is_error),
            elapsed_ms,
        });
        result
    }
}

impl InstalledServerTool {
    async fn execute_inner(&self, arguments: Value) -> anyhow::Result<ToolResult> {
        let tool_input = arguments.clone();
        let client = match McpConfiguredClient::for_config(&self.config) {
            Ok(client) => client,
            Err(_) => return Ok(ToolResult::error("MCP module is unavailable")),
        };
        let client_config = host::client_config(&self.config);
        let arguments: serde_json::Map<String, Value> = match client
            .call(
                &client_config,
                tinymcp_bus::names::methods::NORMALIZE_TOOL_ARGUMENTS,
                (Some(arguments),),
            )
            .await
        {
            Ok(arguments) => arguments,
            Err(_) => return Ok(ToolResult::error("invalid arguments for MCP tool")),
        };

        let outcome: tinymcp_bus::ToolCallOutcome = match client
            .call(
                &client_config,
                tinymcp_bus::names::methods::TOOL_CALL,
                (
                    self.presentation.server_id.as_str(),
                    self.presentation.remote_name.as_str(),
                    Value::Object(arguments),
                ),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(_) => return Ok(ToolResult::error("MCP tool call failed")),
        };
        let rendered: String = match client
            .call::<String>(
                &client_config,
                tinymcp_bus::names::methods::RENDER_TOOL_OUTPUT,
                (tinymcp_bus::RenderToolOutputRequest {
                    result: outcome.rendered.clone(),
                    format: tinymcp_bus::ToolOutputFormat::Llm,
                    prefer_markdown: true,
                },),
            )
            .await
        {
            Ok(rendered) => rendered,
            Err(_) => return Ok(ToolResult::error("MCP tool output is unavailable")),
        };
        let mut result = ToolResult {
            content: vec![ToolContent::Text { text: rendered }],
            is_error: outcome.rendered.is_error,
            ..ToolResult::default()
        };
        result.metadata = Some(
            tinymcp_bus::McpResultEnvelope::new(
                &self.presentation.server_id,
                &self.presentation.remote_name,
                &outcome.rendered,
            )
            .to_metadata(),
        );
        let tool_meta = crate::mcp::ui::decorate::installed_tool(
            &client,
            &client_config,
            &self.presentation.server_id,
            &self.presentation.remote_name,
        )
        .await
        .and_then(|listed| listed.meta);
        crate::mcp::ui::decorate_result(tool_meta.as_ref(), &tool_input, &mut result);
        Ok(result)
    }
}

#[cfg(test)]
#[path = "action_tool_tests.rs"]
mod tests;
