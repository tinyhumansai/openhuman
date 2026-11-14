//! `mcp_call_tool`: forwards one call to a configured static server through
//! TinyMCP's module-owned, classified call operation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{IndirectCall, PermissionLevel, Tool, ToolCallOptions, ToolResult};

use super::{
    bridge_tool_spec, input_module_error_code, input_module_error_message, required_string_arg,
};
use crate::{
    config::Config,
    mcp::{host, module_client::McpConfiguredClient},
    security::SecurityPolicy,
};

/// The host's `mcp_call_tool`, backed by TinyMCP's module-owned static server
/// call operation.
pub struct McpCallTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
    spec: tinymcp_bus::agent_tools::AgentToolSpec,
}

impl McpCallTool {
    fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        let spec = bridge_tool_spec("mcp_call_tool");
        Self {
            config,
            security,
            spec,
        }
    }
}

#[async_trait]
impl Tool for McpCallTool {
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
        PermissionLevel::Execute
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn indirect_target_async(&self, args: &Value) -> anyhow::Result<Option<IndirectCall>> {
        let server = required_string_arg(args, "server")
            .map_err(|_| anyhow::anyhow!("missing MCP server identifier"))?;
        let tool = required_string_arg(args, "tool")
            .map_err(|_| anyhow::anyhow!("missing MCP tool identifier"))?;
        let client = McpConfiguredClient::for_config(&self.config)
            .map_err(|_| anyhow::anyhow!("TinyMCP module is unavailable"))?;
        let server = client
            .normalize_identifier(&server)
            .await
            .map_err(|error| anyhow::anyhow!(input_module_error_message(error)))?;
        let tool = client
            .normalize_identifier(&tool)
            .await
            .map_err(|error| anyhow::anyhow!(input_module_error_message(error)))?;
        let prepared = client
            .prepare_agent_tools(tinymcp_bus::PrepareAgentToolsRequest {
                sources: vec![tinymcp_bus::McpToolSourceSpec {
                    server_id: server.clone(),
                    label: server.clone(),
                    family: server.clone(),
                    display_name: server.clone(),
                    tools: vec![tinymcp_bus::McpTool::new(tool.clone())],
                    default_exposure: tinymcp_bus::McpToolExposure::Direct,
                    direct_tools: vec![tool.clone()],
                }],
            })
            .await
            .map_err(|_| anyhow::anyhow!("TinyMCP target preparation failed"))?;
        let presentation = prepared
            .into_iter()
            .find(|presentation| {
                presentation.server_id == server && presentation.remote_name == tool
            })
            .ok_or_else(|| anyhow::anyhow!("TinyMCP target could not be prepared"))?;
        let mut target = tinytools::ToolSubject::named(presentation.name)
            .with_family(presentation.family)
            .with_permission(PermissionLevel::Execute);
        for tag in presentation.tags {
            target = target.with_tag(tag);
        }
        target.category = Some(tinytools::ToolCategory::Workflow);
        let call = tinytools::IndirectCall::new(target);
        let arguments = args
            .get("arguments")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing MCP tool arguments"))?;
        let arguments = client
            .normalize_tool_arguments(Some(arguments))
            .await
            .map_err(|error| anyhow::anyhow!(input_module_error_message(error)))?;
        Ok(Some(call.with_arguments(Value::Object(arguments))))
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let server = required_string_arg(&args, "server")?;
        let tool = required_string_arg(&args, "tool")?;
        let arguments = args
            .get("arguments")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required `arguments` object"))?;
        let client = match McpConfiguredClient::for_config(&self.config) {
            Ok(client) => client,
            Err(_) => {
                let outcome = unavailable_outcome(&server, &tool);
                return Ok(with_outcome(
                    ToolResult::error("mcp_call_tool failed: TinyMCP module is unavailable"),
                    &outcome,
                ));
            }
        };
        let server = match client.normalize_identifier(&server).await {
            Ok(server) => server,
            Err(error) => {
                let outcome = tinymcp_bus::McpCallOutcome::failed(
                    &server,
                    &tool,
                    tinymcp_bus::McpCallError::new(input_module_error_code(error)),
                );
                return Ok(with_outcome(
                    ToolResult::error(input_module_error_message(error)),
                    &outcome,
                ));
            }
        };
        let tool = match client.normalize_identifier(&tool).await {
            Ok(tool) => tool,
            Err(error) => {
                let outcome = tinymcp_bus::McpCallOutcome::failed(
                    &server,
                    &tool,
                    tinymcp_bus::McpCallError::new(input_module_error_code(error)),
                );
                return Ok(with_outcome(
                    ToolResult::error(input_module_error_message(error)),
                    &outcome,
                ));
            }
        };
        let client_config = host::client_config(&self.config).await;
        let arguments = match client.normalize_tool_arguments(Some(arguments)).await {
            Ok(arguments) => Value::Object(arguments),
            Err(error) => {
                let outcome = tinymcp_bus::McpCallOutcome::failed(
                    &server,
                    &tool,
                    tinymcp_bus::McpCallError::new(input_module_error_code(error)),
                );
                return Ok(with_outcome(
                    ToolResult::error(input_module_error_message(error)),
                    &outcome,
                ));
            }
        };
        let call: tinymcp_bus::ClassifiedStaticToolCall = match client
            .call(
                &client_config,
                tinymcp_bus::names::methods::STATIC_CALL_TOOL_CLASSIFIED,
                (server.as_str(), tool.as_str(), arguments),
            )
            .await
        {
            Ok(call) => call,
            Err(_) => {
                let outcome = unavailable_outcome(&server, &tool);
                return Ok(with_outcome(
                    ToolResult::error("mcp_call_tool failed: TinyMCP module call failed"),
                    &outcome,
                ));
            }
        };

        Ok(classified_result(
            call,
            &server,
            &tool,
            options.prefer_markdown,
        ))
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }
}

/// Builds the host-policy adapter for the generic MCP call tool.
pub fn mcp_call_tool(config: Arc<Config>, security: Arc<SecurityPolicy>) -> McpCallTool {
    McpCallTool::new(config, security)
}

pub(in crate::mcp) fn classified_result(
    call: tinymcp_bus::ClassifiedStaticToolCall,
    server: &str,
    tool: &str,
    prefer_markdown: bool,
) -> ToolResult {
    if !call.outcome.ok {
        let mut result = ToolResult::error(
            call.error_message
                .unwrap_or_else(|| "mcp_call_tool failed".to_owned()),
        );
        result.metadata = serde_json::to_value(call.outcome).ok();
        return result;
    }

    let Some(mut rendered) = call.rendered else {
        let outcome = tinymcp_bus::McpCallOutcome::failed(
            server,
            tool,
            tinymcp_bus::McpCallError::new(tinymcp_bus::errors::SERIALIZATION),
        );
        return with_outcome(ToolResult::error("mcp_call_tool failed"), &outcome);
    };

    let mut result = map_rendered_result(rendered.clone());
    if prefer_markdown && rendered.markdown_formatted.is_none() {
        rendered.markdown_formatted = Some(result.output());
        result.markdown_formatted = rendered.markdown_formatted;
    }

    result.metadata = call
        .envelope
        .and_then(|envelope| serde_json::to_value(envelope).ok())
        .or_else(|| serde_json::to_value(call.outcome).ok());
    result
}

fn map_rendered_result(rendered: tinymcp_bus::McpToolResult) -> ToolResult {
    ToolResult {
        content: rendered
            .content
            .into_iter()
            .map(|block| match block {
                tinymcp_bus::McpToolContent::Text { text } => tinytools::ToolContent::Text { text },
                tinymcp_bus::McpToolContent::Json { data } => tinytools::ToolContent::Json { data },
                other => tinytools::ToolContent::Json {
                    data: serde_json::to_value(other).unwrap_or(Value::Null),
                },
            })
            .collect(),
        is_error: rendered.is_error,
        markdown_formatted: rendered.markdown_formatted,
        ..ToolResult::default()
    }
}

fn unavailable_outcome(server: &str, tool: &str) -> tinymcp_bus::McpCallOutcome {
    tinymcp_bus::McpCallOutcome::failed(
        server,
        tool,
        tinymcp_bus::McpCallError::new(tinymcp_bus::errors::BUS),
    )
}

fn with_outcome(mut result: ToolResult, outcome: &tinymcp_bus::McpCallOutcome) -> ToolResult {
    result.metadata = serde_json::to_value(outcome).ok();
    result
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "call_target_tests.rs"]
mod target_tests;
