//! Host adapters for TinyMCP's generic configured-server bridge.
//!
//! TinyMCP owns the generic bridge declarations, transport calls, rendering,
//! secret scrubbing, and call classification. This module keeps OpenHuman's
//! act gate and adapts the returned bus vocabulary to `tinytools`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::{IndirectCall, PermissionLevel, Tool, ToolCallOptions, ToolResult};

use crate::{
    config::Config,
    mcp::{host, module_client::McpConfiguredClient},
    security::{SecurityPolicy, ToolOperation},
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
                ))
            }
        };
        let config = host::client_config(&self.config);
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
                ))
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
                ))
            }
        };
        let server = client
            .normalize_identifier(&server)
            .await
            .map_err(|error| anyhow::anyhow!(input_module_error_message(error)))?;
        let client_config = host::client_config(&self.config);
        let remote: Vec<tinymcp_bus::McpRemoteTool> = match client
            .call(
                &client_config,
                tinymcp_bus::names::methods::STATIC_LIST_TOOLS,
                (server.as_str(),),
            )
            .await
        {
            Ok(remote) => remote,
            Err(_) => {
                return Ok(ToolResult::error(
                    "mcp_list_tools failed: module call failed",
                ))
            }
        };
        let remote = match client
            .scrub_static_tools(&client_config, &server, remote)
            .await
        {
            Ok(remote) => remote,
            Err(_) => {
                return Ok(ToolResult::error(
                    "mcp_list_tools failed: metadata scrubbing failed",
                ))
            }
        };

        let mut display = Vec::with_capacity(remote.len());
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
                    ))
                }
            };
            display.push((tool, projected));
        }

        let policy_tools = display
            .iter()
            .map(|(tool, projected)| tinymcp_bus::McpTool {
                name: tool.name.clone(),
                description: projected.description.clone().or(projected.title.clone()),
                input_schema: tool.input_schema.clone(),
                meta: None,
            })
            .collect();
        let safe = crate::mcp::registry::tools_safe_for_agent(&server, policy_tools);
        let safe_names: std::collections::HashSet<_> =
            safe.into_iter().map(|tool| tool.name).collect();
        let payload = display
            .into_iter()
            .filter(|(tool, _)| safe_names.contains(&tool.name))
            .map(|(tool, projected)| {
                json!({
                    "name": tool.name,
                    "title": projected.title,
                    "description": projected.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
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

fn bridge_tool_spec(name: &str) -> tinymcp_bus::agent_tools::AgentToolSpec {
    tinymcp_bus::agent_tools::bridge_tool_specs()
        .into_iter()
        .find(|spec| spec.name == name)
        .expect("TinyMCP bridge contract includes the requested tool")
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
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(|error| anyhow::anyhow!(error))?;

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
        let client_config = host::client_config(&self.config);
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

pub(super) fn classified_result(
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

pub(super) fn input_module_error_code(error: tinybus::module::ModuleCallError) -> &'static str {
    use tinybus::module::ModuleCallError;
    match error {
        ModuleCallError::Unavailable | ModuleCallError::IncompatibleContract => {
            tinymcp_bus::errors::BUS
        }
        ModuleCallError::TransportFailed => tinymcp_bus::errors::TRANSPORT,
        ModuleCallError::ModuleFault => tinymcp_bus::errors::BUS,
    }
}

pub(super) fn input_module_error_message(error: tinybus::module::ModuleCallError) -> &'static str {
    use tinybus::module::ModuleCallError;
    match error {
        ModuleCallError::Unavailable => "TinyMCP module is unavailable",
        ModuleCallError::IncompatibleContract => "TinyMCP contract is incompatible",
        ModuleCallError::TransportFailed => "TinyMCP transport failed",
        ModuleCallError::ModuleFault => "TinyMCP execution failed",
    }
}

fn with_outcome(mut result: ToolResult, outcome: &tinymcp_bus::McpCallOutcome) -> ToolResult {
    result.metadata = serde_json::to_value(outcome).ok();
    result
}

/// Reads a required identifier without applying MCP-owned normalization.
fn required_string_arg(args: &Value, key: &str) -> anyhow::Result<String> {
    let missing = || anyhow::anyhow!("missing required `{key}`");
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(missing)
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mcp_target_tests.rs"]
mod target_tests;
