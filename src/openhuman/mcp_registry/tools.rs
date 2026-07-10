//! LLM-callable wrappers over the `mcp_registry` client surface.
//!
//! These expose the installed-MCP-servers registry to the agent: search the
//! catalog, inspect a server, list installed servers and their connection
//! status, connect/disconnect, call a tool on a connected server, and get
//! AI config help. Thin shims over [`crate::openhuman::mcp_registry::ops`].
//!
//! Discovery/observe/connect/call tools are default-ON. The persistent
//! `mcp_registry_install` / `mcp_registry_uninstall` mutators (write installed
//! state + secrets) ship default-OFF via `tools/user_filter.rs`
//! (`mcp_manage` toggle).
//!
//! NOTE: the `mcp_setup_*` setup-agent tools and the generic `mcp_list_servers`
//! / `mcp_call_tool` bridge tools already exist elsewhere; these `mcp_registry_*`
//! tools are the distinct installed-registry surface and do not duplicate them.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;

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

/// Search the MCP registry catalog.
pub struct McpRegistrySearchTool {
    config: Arc<Config>,
}
impl McpRegistrySearchTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistrySearchTool {
    fn name(&self) -> &str {
        "mcp_registry_search"
    }
    fn description(&self) -> &str {
        "Search the MCP server registry catalog by `query`, optionally filtered by \
         `transport` (\"stdio\" | \"hosted\" | \"all\"), paginated by `page` / \
         `page_size`. Use to discover installable MCP servers."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "transport": { "type": "string", "enum": ["stdio", "hosted", "all"] },
                "page": { "type": "integer", "minimum": 1 },
                "page_size": { "type": "integer", "minimum": 1 }
            }
        })
    }
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
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Get one registry server by qualified name.
pub struct McpRegistryGetTool {
    config: Arc<Config>,
}
impl McpRegistryGetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryGetTool {
    fn name(&self) -> &str {
        "mcp_registry_get"
    }
    fn description(&self) -> &str {
        "Get one MCP registry server's detail by `qualified_name`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "qualified_name": { "type": "string" } },
            "required": ["qualified_name"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        emit!(
            ops::mcp_clients_registry_get(&self.config, qn).await,
            "mcp_registry_get"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List installed MCP servers.
pub struct McpRegistryInstalledListTool {
    config: Arc<Config>,
}
impl McpRegistryInstalledListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryInstalledListTool {
    fn name(&self) -> &str {
        "mcp_registry_installed_list"
    }
    fn description(&self) -> &str {
        "List the MCP servers currently installed for this user."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_installed_list(&self.config).await,
            "mcp_registry_installed_list"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Connection status of installed MCP servers.
pub struct McpRegistryStatusTool {
    config: Arc<Config>,
}
impl McpRegistryStatusTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryStatusTool {
    fn name(&self) -> &str {
        "mcp_registry_status"
    }
    fn description(&self) -> &str {
        "Report the connection status of installed MCP servers."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_status(&self.config).await,
            "mcp_registry_status"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List the tools advertised by a connected MCP server (discovery).
pub struct McpRegistryListToolsTool;
#[async_trait]
impl Tool for McpRegistryListToolsTool {
    fn name(&self) -> &str {
        "mcp_registry_list_tools"
    }
    fn description(&self) -> &str {
        "List the tools (name, description, input schema) exposed by a \
         connected MCP server, given its `server_id`. Use this to discover \
         what a connected server can do before calling `mcp_registry_tool_call`. \
         The server must already be connected (see `mcp_registry_status` / \
         `mcp_registry_connect`)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_list_tools(sid).await,
            "mcp_registry_list_tools"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Connect an installed MCP server.
pub struct McpRegistryConnectTool {
    config: Arc<Config>,
}
impl McpRegistryConnectTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryConnectTool {
    fn name(&self) -> &str {
        "mcp_registry_connect"
    }
    fn description(&self) -> &str {
        "Connect (spawn + handshake) an installed MCP server by `server_id`, \
         returning its tools."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_connect(&self.config, sid).await,
            "mcp_registry_connect"
        )
    }
}

/// Disconnect an MCP server.
pub struct McpRegistryDisconnectTool;
#[async_trait]
impl Tool for McpRegistryDisconnectTool {
    fn name(&self) -> &str {
        "mcp_registry_disconnect"
    }
    fn description(&self) -> &str {
        "Disconnect (stop) a connected MCP server by `server_id`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_disconnect(sid).await,
            "mcp_registry_disconnect"
        )
    }
}

/// Call a tool on a connected MCP server.
pub struct McpRegistryToolCallTool;
#[async_trait]
impl Tool for McpRegistryToolCallTool {
    fn name(&self) -> &str {
        "mcp_registry_tool_call"
    }
    fn description(&self) -> &str {
        "Invoke a tool on a connected MCP server: `server_id` + `tool_name` + \
         `arguments` object."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "server_id": { "type": "string" },
                "tool_name": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["server_id", "tool_name"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        let tool_name = req_str(&args, "tool_name")?;
        let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
        emit!(
            ops::mcp_clients_tool_call(sid, tool_name, arguments).await,
            "mcp_registry_tool_call"
        )
    }
}

/// AI config assistance for an MCP server.
pub struct McpRegistryConfigAssistTool {
    config: Arc<Config>,
}
impl McpRegistryConfigAssistTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryConfigAssistTool {
    fn name(&self) -> &str {
        "mcp_registry_config_assist"
    }
    fn description(&self) -> &str {
        "Get AI guidance for configuring an MCP server (`qualified_name`) given a \
         `user_message`; returns a reply and suggested env vars."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": { "type": "string" },
                "user_message": { "type": "string" }
            },
            "required": ["qualified_name", "user_message"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        let msg = req_str(&args, "user_message")?;
        emit!(
            ops::mcp_clients_config_assist(&self.config, qn, msg, None).await,
            "mcp_registry_config_assist"
        )
    }
}

/// Install an MCP server (persists install + env). Default-OFF.
pub struct McpRegistryInstallTool {
    config: Arc<Config>,
}
impl McpRegistryInstallTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryInstallTool {
    fn name(&self) -> &str {
        "mcp_registry_install"
    }
    fn description(&self) -> &str {
        "Install an MCP server (`qualified_name`) with an `env` map and optional \
         `config`. Persists the install + secrets. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": { "type": "string" },
                "env": { "type": "object", "additionalProperties": { "type": "string" } },
                "config": { "type": "object" }
            },
            "required": ["qualified_name"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        let env: HashMap<String, String> = args
            .get("env")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| anyhow::anyhow!("mcp_registry_install: invalid env: {e}"))?
            .unwrap_or_default();
        let config_value = args.get("config").cloned();
        emit!(
            ops::mcp_clients_install(&self.config, qn, env, config_value).await,
            "mcp_registry_install"
        )
    }
}

/// Uninstall an MCP server. Default-OFF.
pub struct McpRegistryUninstallTool {
    config: Arc<Config>,
}
impl McpRegistryUninstallTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryUninstallTool {
    fn name(&self) -> &str {
        "mcp_registry_uninstall"
    }
    fn description(&self) -> &str {
        "Uninstall an installed MCP server by `server_id`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_uninstall(&self.config, sid).await,
            "mcp_registry_uninstall"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(
            McpRegistrySearchTool::new(cfg()).name(),
            "mcp_registry_search"
        );
        assert_eq!(
            McpRegistrySearchTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            McpRegistryConnectTool::new(cfg()).permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            McpRegistryToolCallTool.permission_level(),
            PermissionLevel::Execute
        );
        // Discovery tool: read-only, names match.
        assert_eq!(McpRegistryListToolsTool.name(), "mcp_registry_list_tools");
        assert_eq!(
            McpRegistryListToolsTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            McpRegistryInstallTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(McpRegistrySearchTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn get_requires_qualified_name() {
        let err = McpRegistryGetTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing qualified_name");
        assert!(err.to_string().contains("qualified_name"));
    }

    #[tokio::test]
    async fn list_tools_requires_server_id() {
        let err = McpRegistryListToolsTool
            .execute(json!({}))
            .await
            .expect_err("missing server_id");
        assert!(err.to_string().contains("server_id"));
    }

    #[tokio::test]
    async fn list_tools_errors_for_unconnected_server() {
        // A server_id that is not in the live connection map surfaces a
        // "connect first" hint rather than an empty success.
        let err = McpRegistryListToolsTool
            .execute(json!({ "server_id": "definitely-not-connected-uuid" }))
            .await
            .expect_err("unconnected server must error");
        assert!(
            err.to_string().contains("not connected"),
            "expected connect-first hint, got: {err}"
        );
    }
}
