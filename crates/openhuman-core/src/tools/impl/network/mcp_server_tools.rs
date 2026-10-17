//! Every tool on a configured (`[[mcp_client.servers]]` / `AgentSpec::mcp`)
//! server as its own agent tool.
//!
//! The generic bridge (`mcp_list_tools` + `mcp_call_tool`) makes a model
//! discover a server's tools and route each call through one proxy. These
//! tools are the direct form: `mcp_<server>_<tool>`, with the remote schema,
//! deferred by default so `tool_search` finds them, or sent every turn when
//! the server says `expose = "direct"` / lists them in `direct_tools`.
//!
//! They are built from tinymcp's persistent tool cache, never from the
//! network, so assembling a tool list costs no round trip. A server with
//! nothing cached yet is listed in the background and its tools appear on the
//! next build; until then the generic bridge still reaches it.
//!
//! Policy stays here: the act gate, the prompt-injection scan on remote
//! definitions, and the secret scrubber on output, exactly as `mcp_call_tool`
//! applies them.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{PermissionLevel, Tool, ToolCategory, ToolExposure, ToolResult};

use crate::config::Config;
use crate::security::{SecurityPolicy, ToolOperation};

/// Builds host adapters from TinyMCP-prepared declarations. Catalog retrieval,
/// naming, schema normalization, and execution remain module-owned.
pub fn configured_server_tools_from_presentations(
    config: &Config,
    presentations: Vec<tinymcp_bus::McpToolPresentation>,
    security: &Arc<SecurityPolicy>,
) -> Vec<Box<dyn Tool>> {
    presentations
        .into_iter()
        .filter_map(|presentation| {
            let server = config
                .mcp_client
                .servers
                .iter()
                .find(|server| server.server.name == presentation.server_id)?;
            Some(Box::new(ModuleConfiguredMcpServerTool {
                presentation,
                config: Arc::new(config.clone()),
                server: server.server.clone(),
                security: Arc::clone(security),
            }) as Box<dyn Tool>)
        })
        .collect()
}

struct ModuleConfiguredMcpServerTool {
    presentation: tinymcp_bus::McpToolPresentation,
    config: Arc<Config>,
    server: tinymcp_bus::McpServerConfig,
    security: Arc<SecurityPolicy>,
}

#[async_trait]
impl Tool for ModuleConfiguredMcpServerTool {
    fn name(&self) -> &str {
        &self.presentation.name
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

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, self.name())
            .map_err(|error| anyhow::anyhow!(error))?;
        let current = crate::config::ops::reload_config_from_paths(
            &self.config.config_path,
            &self.config.workspace_dir,
        )
        .await
        .map_err(|_| anyhow::anyhow!("could not reload MCP configuration"))?;
        let current_server = current
            .mcp_client
            .servers
            .iter()
            .find(|server| server.server.name == self.presentation.server_id)
            .map(|server| &server.server)
            .ok_or_else(|| anyhow::anyhow!("MCP server is no longer configured"))?;
        if current_server != &self.server {
            anyhow::bail!("MCP server configuration changed; rebuild tools before calling");
        }

        let client = crate::mcp::module_client::McpConfiguredClient::for_config(&self.config)
            .map_err(|_| anyhow::anyhow!("TinyMCP module is unavailable"))?;
        let client_config = crate::mcp::host::client_config(&self.config);
        let live: Vec<tinymcp_bus::McpRemoteTool> = client
            .call(
                &client_config,
                tinymcp_bus::names::methods::STATIC_LIST_TOOLS,
                (self.presentation.server_id.as_str(),),
            )
            .await
            .map_err(|_| anyhow::anyhow!("MCP tool is unavailable"))?;
        let live = client
            .scrub_static_tools(&client_config, &self.presentation.server_id, live)
            .await
            .map_err(|_| anyhow::anyhow!("MCP tool metadata is unavailable"))?;
        let tool_meta = live
            .iter()
            .find(|tool| tool.name == self.presentation.remote_name)
            .and_then(|tool| tool.meta.clone());
        let safe = crate::mcp::registry::tools_safe_for_agent(
            &self.presentation.server_id,
            live.into_iter()
                .map(|tool| tinymcp_bus::McpTool {
                    name: tool.name,
                    description: tool.description.or(tool.title),
                    input_schema: tool.input_schema,
                    meta: None,
                })
                .collect(),
        );
        if !safe
            .iter()
            .any(|tool| tool.name == self.presentation.remote_name)
        {
            anyhow::bail!("MCP tool is no longer available or safe");
        }
        let arguments = client
            .normalize_tool_arguments(Some(args))
            .await
            .map_err(|error| anyhow::anyhow!(super::mcp::input_module_error_message(error)))?;
        let tool_input = Value::Object(arguments.clone());
        let call: tinymcp_bus::ClassifiedStaticToolCall = client
            .call_confidential(
                &client_config,
                tinymcp_bus::names::methods::STATIC_CALL_TOOL_CLASSIFIED,
                (
                    self.presentation.server_id.as_str(),
                    self.presentation.remote_name.as_str(),
                    Value::Object(arguments),
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("MCP tool call failed"))?;
        let mut result = super::mcp::classified_result(
            call,
            &self.presentation.server_id,
            &self.presentation.remote_name,
            false,
        );
        crate::mcp::ui::decorate_result(tool_meta.as_ref(), &tool_input, &mut result);
        Ok(result)
    }
}

#[cfg(test)]
#[path = "mcp_server_tools_tests.rs"]
mod tests;
