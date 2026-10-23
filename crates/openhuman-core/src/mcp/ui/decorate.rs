//! Turning a tool result's `mcp_result` envelope into the presentation the
//! chat surface reads, on every path an MCP tool call takes.
//!
//! The envelope is replaced, never forwarded: it carries the server's raw
//! payload, embedded widget HTML included, and only the bounded presentation
//! may reach events and transcripts. A call that offered neither a widget nor
//! a link keeps no metadata at all.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolInjectedArgument,
    ToolPolicy, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};

use crate::config::Config;
use crate::mcp::host;
use crate::mcp::module_client::McpConfiguredClient;

use super::resolve::{presentation_from_view, view_from_envelope};
use super::types::MCP_RESULT_KIND;

/// Whether `metadata` is an `mcp_result` envelope.
#[must_use]
pub fn is_envelope(metadata: &Value) -> bool {
    metadata.get("kind").and_then(Value::as_str) == Some(MCP_RESULT_KIND)
}

/// Replaces an `mcp_result` envelope in `result.metadata` with its
/// presentation, or with nothing. Any other metadata is left alone.
pub fn decorate_result(tool_meta: Option<&Value>, tool_input: &Value, result: &mut ToolResult) {
    let Some(metadata) = result.metadata.as_ref() else {
        return;
    };
    if !is_envelope(metadata) {
        return;
    }
    let Some(view) = view_from_envelope(
        metadata,
        tool_meta.cloned(),
        tool_input.clone(),
        &result.text(),
    ) else {
        result.metadata = None;
        return;
    };
    result.metadata = presentation_from_view(&view).map(|presentation| presentation.to_metadata());
}

/// Where a wrapped tool's descriptor `_meta` comes from, keyed by the server
/// and tool the envelope names.
pub enum MetaLookup {
    /// No descriptor `_meta` is available; only the result's own metadata is
    /// read.
    Absent,
    /// Configured servers: the module's live listing.
    Configured(Arc<Config>),
}

impl MetaLookup {
    async fn tool_meta(&self, server: &str, tool: &str) -> Option<Value> {
        match self {
            Self::Absent => None,
            Self::Configured(config) => configured_tool_meta(config, server, tool).await,
        }
    }
}

async fn configured_tool_meta(config: &Config, server: &str, tool: &str) -> Option<Value> {
    let client = McpConfiguredClient::for_config(config).ok()?;
    let client_config = host::client_config(config);
    let listed: Vec<tinymcp_bus::McpRemoteTool> = client
        .call(
            &client_config,
            tinymcp_bus::names::methods::STATIC_LIST_TOOLS,
            (server,),
        )
        .await
        .ok()?;
    listed
        .into_iter()
        .find(|remote| remote.name == tool)
        .and_then(|remote| remote.meta)
}

/// An installed server's listing of `tool`, when it passes the agent safety
/// scan.
pub(crate) async fn installed_tool(
    client: &McpConfiguredClient,
    client_config: &tinymcp_bus::McpClientConfig,
    server_id: &str,
    tool: &str,
) -> Option<tinymcp_bus::McpTool> {
    let listed: Vec<tinymcp_bus::McpTool> = client
        .call(
            client_config,
            tinymcp_bus::names::methods::LIST_TOOLS,
            (server_id,),
        )
        .await
        .map_err(|error| {
            tracing::debug!(
                server_id,
                tool,
                ?error,
                "[mcp_ui] could not list installed tools"
            );
        })
        .ok()?;
    let candidate = listed.into_iter().find(|listed| listed.name == tool)?;
    crate::mcp::registry::tools_safe_for_agent(server_id, vec![candidate])
        .await
        .into_iter()
        .next()
}

/// The descriptor `_meta` of an installed server's `tool`.
pub(crate) async fn installed_tool_meta(
    config: &Config,
    server_id: &str,
    tool: &str,
) -> Option<Value> {
    let client = McpConfiguredClient::for_config(config).ok()?;
    let client_config = host::client_config(config);
    installed_tool(&client, &client_config, server_id, tool)
        .await
        .and_then(|listed| listed.meta)
}

/// Which part of a call's arguments the remote tool received.
#[derive(Debug, Clone, Copy)]
pub enum ToolInput {
    /// The arguments are the remote tool's.
    Direct,
    /// The remote arguments sit under this key (`mcp_call_tool`).
    Nested(&'static str),
}

impl ToolInput {
    fn of(self, args: &Value) -> Value {
        match self {
            Self::Direct => args.clone(),
            Self::Nested(key) => args.get(key).cloned().unwrap_or(Value::Null),
        }
    }
}

/// An MCP tool whose results carry a presentation instead of the raw
/// envelope.
pub struct UiAwareTool {
    inner: Box<dyn Tool>,
    lookup: MetaLookup,
    input: ToolInput,
}

impl UiAwareTool {
    #[must_use]
    pub fn new(inner: Box<dyn Tool>, lookup: MetaLookup, input: ToolInput) -> Self {
        Self {
            inner,
            lookup,
            input,
        }
    }

    async fn finish(
        &self,
        args: &Value,
        outcome: anyhow::Result<ToolResult>,
    ) -> anyhow::Result<ToolResult> {
        let mut result = outcome?;
        let Some(metadata) = result.metadata.as_ref().filter(|value| is_envelope(value)) else {
            return Ok(result);
        };
        let server = metadata
            .get("server")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let tool = metadata
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let tool_meta = self.lookup.tool_meta(&server, &tool).await;
        decorate_result(tool_meta.as_ref(), &self.input.of(args), &mut result);
        if result.metadata.as_ref().is_some_and(is_envelope) {
            result.metadata = None;
        }
        Ok(result)
    }
}

#[async_trait]
impl Tool for UiAwareTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let outcome = self.inner.execute(args.clone()).await;
        self.finish(&args, outcome).await
    }
    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let outcome = self.inner.execute_with_options(args.clone(), options).await;
        self.finish(&args, outcome).await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let outcome = self
            .inner
            .execute_with_context(args.clone(), options, context)
            .await;
        self.finish(&args, outcome).await
    }
    fn policy(&self) -> ToolPolicy {
        self.inner.policy()
    }
    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.inner.injected_arguments()
    }
    fn supports_markdown(&self) -> bool {
        self.inner.supports_markdown()
    }
    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }
    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.inner.permission_level_with_args(args)
    }
    fn scope(&self) -> ToolScope {
        self.inner.scope()
    }
    fn category(&self) -> ToolCategory {
        self.inner.category()
    }
    fn exposure(&self) -> ToolExposure {
        self.inner.exposure()
    }
    fn family(&self) -> Option<&str> {
        self.inner.family()
    }
    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.inner.is_concurrency_safe(args)
    }
    fn external_effect(&self) -> bool {
        self.inner.external_effect()
    }
    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.inner.external_effect_with_args(args)
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.inner.max_result_size_chars()
    }
    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.inner.timeout_policy(args)
    }
    fn host_extension(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.inner.host_extension()
    }
    fn host_call_extension(&self, args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        self.inner.host_call_extension(args)
    }
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn display_label(&self, args: &Value) -> Option<String> {
        self.inner.display_label(args)
    }
    fn display_detail(&self, args: &Value) -> Option<String> {
        self.inner.display_detail(args)
    }
    fn return_direct(&self) -> bool {
        self.inner.return_direct()
    }
}

#[cfg(test)]
#[path = "decorate_tests.rs"]
mod tests;
