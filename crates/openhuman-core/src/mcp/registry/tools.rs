//! LLM-callable wrappers over the `mcp::registry` client surface.
//!
//! These expose the installed-MCP-servers registry to the agent: search the
//! catalog, inspect a server, list installed servers and their connection
//! status, connect/disconnect, and call a tool on a connected server. Thin
//! shims over [`crate::mcp::registry::ops`].
//!
//! What each tool *is* — its name, description, schema, effect and whether it
//! is deferred — is the contract's (`tinymcp_bus::agent_tools::RegistryTool`),
//! so the identity a model and a cached prompt see is defined once. What is
//! left here is host policy: the permission level an effect maps to, the
//! exposure, and running the call against this workspace's registry.
//!
//! Discovery/observe/connect/call tools are default-ON. The persistent
//! `mcp_registry_uninstall` mutator ships default-OFF via
//! `tools/user_filter.rs` (`mcp_manage` toggle). There is no install tool: a
//! server is declared by the user in `mcp.json`, never by an agent from a
//! catalog listing.
//!
//! NOTE: the generic `mcp_list_servers` / `mcp_call_tool` bridge tools already
//! exist elsewhere; these `mcp_registry_*` tools are the distinct
//! installed-registry surface and do not duplicate them.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::Config;
use tinymcp_bus::agent_tools::{
    normalize_tool_arguments, AgentToolEffect, AgentToolSpec, RegistryTool,
};
use tinytools::{PermissionLevel, Tool, ToolExposure, ToolResult};

use super::ops;

/// The permission level this host requires for a tool's effect.
///
/// An effect the contract adds later maps to the strictest level until this
/// host decides otherwise.
pub(crate) fn permission_for(effect: AgentToolEffect) -> PermissionLevel {
    match effect {
        AgentToolEffect::Read => PermissionLevel::ReadOnly,
        AgentToolEffect::Execute => PermissionLevel::Execute,
        _ => PermissionLevel::Write,
    }
}

/// How this host exposes a spec: deferred specs are found by search.
pub(crate) fn exposure_for(spec: &AgentToolSpec) -> ToolExposure {
    if spec.deferred {
        ToolExposure::Deferred
    } else {
        ToolExposure::Direct
    }
}

/// The spec-derived half of a registry tool's `Tool` impl.
macro_rules! spec_metadata {
    () => {
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
            permission_for(self.spec.effect)
        }
        fn exposure(&self) -> ToolExposure {
            exposure_for(&self.spec)
        }
        // Observing is safe to run beside other calls; acting is not.
        fn is_concurrency_safe(&self, _args: &Value) -> bool {
            self.spec.effect == AgentToolEffect::Read
        }
    };
}

/// A registry tool struct holding its config and its contract spec.
macro_rules! registry_tool {
    ($(#[$doc:meta])* $ty:ident => $kind:ident) => {
        $(#[$doc])*
        pub struct $ty {
            config: Arc<Config>,
            spec: AgentToolSpec,
        }
        impl $ty {
            /// Builds the tool over `config`.
            #[must_use]
            pub fn new(config: Arc<Config>) -> Self {
                Self {
                    config,
                    spec: RegistryTool::$kind.spec(),
                }
            }
        }
    };
}

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

fn req_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

registry_tool! {
    /// Search the MCP registry catalog.
    McpRegistrySearchTool => Search
}
#[async_trait]
impl Tool for McpRegistrySearchTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::to_string);
        let page = args.get("page").and_then(Value::as_u64).map(|v| v as u32);
        let page_size = args
            .get("page_size")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        let transport = args
            .get("transport")
            .and_then(Value::as_str)
            .map(str::to_string);
        emit!(
            ops::mcp_clients_registry_search(&self.config, query, transport, page, page_size).await,
            "mcp_registry_search"
        )
    }
}

registry_tool! {
    /// Get one registry server by qualified name.
    McpRegistryGetTool => Get
}
#[async_trait]
impl Tool for McpRegistryGetTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        emit!(
            ops::mcp_clients_registry_get(&self.config, qn).await,
            "mcp_registry_get"
        )
    }
}

registry_tool! {
    /// List installed MCP servers.
    McpRegistryInstalledListTool => InstalledList
}
#[async_trait]
impl Tool for McpRegistryInstalledListTool {
    spec_metadata!();

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_installed_list(&self.config).await,
            "mcp_registry_installed_list"
        )
    }
}

registry_tool! {
    /// Connection status of installed MCP servers.
    McpRegistryStatusTool => Status
}
#[async_trait]
impl Tool for McpRegistryStatusTool {
    spec_metadata!();

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_status(&self.config).await,
            "mcp_registry_status"
        )
    }
}

registry_tool! {
    /// List the tools advertised by a connected MCP server (discovery).
    McpRegistryListToolsTool => ListTools
}
#[async_trait]
impl Tool for McpRegistryListToolsTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_list_tools(&self.config, sid, ops::Caller::Agent).await,
            "mcp_registry_list_tools"
        )
    }
}

registry_tool! {
    /// Connect an installed MCP server.
    McpRegistryConnectTool => Connect
}
#[async_trait]
impl Tool for McpRegistryConnectTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_connect(&self.config, sid).await,
            "mcp_registry_connect"
        )
    }
}

registry_tool! {
    /// Disconnect an MCP server.
    McpRegistryDisconnectTool => Disconnect
}
#[async_trait]
impl Tool for McpRegistryDisconnectTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_disconnect(&self.config, sid).await,
            "mcp_registry_disconnect"
        )
    }
}

registry_tool! {
    /// Call a tool on a connected MCP server.
    McpRegistryToolCallTool => ToolCall
}
#[async_trait]
impl Tool for McpRegistryToolCallTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        let tool_name = req_str(&args, "tool_name")?;
        // Tolerant at execution, unchanged in the schema: an object that
        // arrived JSON-encoded in a string is decoded, and anything that
        // cannot hold one is refused here, naming what arrived.
        let arguments = normalize_tool_arguments(args.get("arguments").cloned())
            .map(Value::Object)
            .map_err(|error| anyhow::anyhow!("mcp_registry_tool_call: {error}"))?;
        let outcome = ops::mcp_clients_tool_call(
            &self.config,
            sid.clone(),
            tool_name.clone(),
            arguments.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("mcp_registry_tool_call: {e}"))?;
        let mut result = ToolResult::success(serde_json::to_string(&outcome.value)?);
        let raw = outcome.value.get("result").filter(|raw| raw.is_object());
        if let Some(raw) = raw {
            let tool_meta = match crate::mcp::host::for_config(&self.config) {
                Ok(service) => {
                    service
                        .dynamic()
                        .connections()
                        .tool_meta(&sid, &tool_name)
                        .await
                }
                Err(_) => None,
            };
            let view = crate::mcp::ui::resolve::view_from_raw_result(
                &sid, &tool_name, tool_meta, arguments, raw,
            );
            result.metadata = crate::mcp::ui::resolve::presentation_from_view(&view)
                .map(|presentation| presentation.to_metadata());
        }
        Ok(result)
    }
}

registry_tool! {
    /// Uninstall an MCP server. Default-OFF.
    McpRegistryUninstallTool => Uninstall
}
#[async_trait]
impl Tool for McpRegistryUninstallTool {
    spec_metadata!();

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_uninstall(&self.config, sid).await,
            "mcp_registry_uninstall"
        )
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
