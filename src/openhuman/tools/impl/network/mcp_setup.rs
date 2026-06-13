//! Agent-facing tool wrappers around `mcp_registry::setup_ops`.
//!
//! Six thin tools the `mcp_setup` sub-agent uses to walk the user
//! through installing an MCP server. They are intentionally simple —
//! the real logic lives in
//! [`crate::openhuman::mcp_registry::setup_ops`]; these structs only
//! marshall args ↔ `serde_json::Value` and turn `RpcOutcome` into a
//! `ToolResult`.
//!
//! Secret values **never** pass through these tools. `request_secret`
//! returns an opaque `secret://<hex>` ref; the agent stores the ref and
//! passes it back into `test_connection` / `install_and_connect` which
//! resolve it inside the core process.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::mcp_registry::setup_ops;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn outcome_to_result(
    outcome: Result<crate::rpc::RpcOutcome<Value>, String>,
) -> anyhow::Result<ToolResult> {
    match outcome {
        Ok(out) => Ok(ToolResult::json(out.value)),
        Err(err) => Ok(ToolResult::error(err)),
    }
}

fn read_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string `{key}`"))
}

fn read_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn read_u32_opt(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(Value::as_u64).map(|v| v as u32)
}

fn read_str_map(args: &Value, key: &str) -> Result<HashMap<String, String>, String> {
    let v = args
        .get(key)
        .ok_or_else(|| format!("missing required object `{key}`"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| format!("`{key}` must be an object"))?;
    let mut out = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        let s = v
            .as_str()
            .ok_or_else(|| format!("`{key}[{k}]` must be a string"))?;
        out.insert(k.clone(), s.to_string());
    }
    Ok(out)
}

// ── mcp_setup_search ─────────────────────────────────────────────────────────

pub struct McpSetupSearchTool {
    config: Arc<Config>,
}

impl McpSetupSearchTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for McpSetupSearchTool {
    fn name(&self) -> &str {
        "mcp_setup_search"
    }

    fn description(&self) -> &str {
        "Search all enabled MCP server registries (Smithery + modelcontextprotocol/registry). \
         Returns merged results tagged with the upstream `source`. Use to discover candidate \
         servers by keyword (e.g. 'notion', 'filesystem', 'github')."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Free-text search query." },
                "page": { "type": "integer", "description": "1-based page number (default 1)." },
                "page_size": { "type": "integer", "description": "Results per page (default 20)." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let query = read_str_opt(&args, "query");
        let page = read_u32_opt(&args, "page");
        let page_size = read_u32_opt(&args, "page_size");
        outcome_to_result(setup_ops::mcp_setup_search(&self.config, query, page, page_size).await)
    }
}

// ── mcp_setup_get ────────────────────────────────────────────────────────────

pub struct McpSetupGetTool {
    config: Arc<Config>,
}

impl McpSetupGetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for McpSetupGetTool {
    fn name(&self) -> &str {
        "mcp_setup_get"
    }

    fn description(&self) -> &str {
        "Fetch full detail for one MCP server, including the `required_env_keys` array derived \
         from its connection schema. Use to plan which secrets to request from the user."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": {
                    "type": "string",
                    "description": "Registry qualified name (e.g. `@notion/server-notion`). \
                                    May be prefixed with `<source>::` to pin a registry."
                }
            },
            "required": ["qualified_name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let qualified_name = match read_str(&args, "qualified_name") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        outcome_to_result(setup_ops::mcp_setup_get(&self.config, qualified_name).await)
    }
}

// ── mcp_setup_request_secret ─────────────────────────────────────────────────

pub struct McpSetupRequestSecretTool;

impl McpSetupRequestSecretTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for McpSetupRequestSecretTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for McpSetupRequestSecretTool {
    fn name(&self) -> &str {
        "mcp_setup_request_secret"
    }

    fn description(&self) -> &str {
        "Ask the user to provide a secret value (API key, OAuth token, etc.) via a native UI \
         prompt. Returns an opaque ref like `secret://<hex>`. The raw value never enters this \
         agent's context — only the ref does. Pass the ref back into `mcp_setup_test_connection` \
         or `mcp_setup_install_and_connect`. Blocks for up to 5 minutes waiting on the user."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key_name": {
                    "type": "string",
                    "description": "Env-var name (e.g. `NOTION_API_KEY`). Shown to the user as the field label."
                },
                "prompt": {
                    "type": "string",
                    "description": "Plain-English explanation the user sees, e.g. 'Paste your Notion integration token from notion.so/my-integrations.'"
                }
            },
            "required": ["key_name", "prompt"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // No filesystem / network — purely an IPC handshake. ReadOnly is
        // wrong (it's user-input), but Write is too strong. The harness
        // gates by `< Execute`, so ReadOnly keeps the agent able to call
        // it.
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let key_name = match read_str(&args, "key_name") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let prompt = match read_str(&args, "prompt") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        outcome_to_result(setup_ops::mcp_setup_request_secret(key_name, prompt).await)
    }
}

// ── mcp_setup_test_connection ────────────────────────────────────────────────

pub struct McpSetupTestConnectionTool {
    config: Arc<Config>,
}

impl McpSetupTestConnectionTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for McpSetupTestConnectionTool {
    fn name(&self) -> &str {
        "mcp_setup_test_connection"
    }

    fn description(&self) -> &str {
        "Dry-run install: spawn the candidate MCP server in a scratch process with the supplied \
         secret refs, list its tools, tear it down. Nothing is persisted. Returns \
         `{ ok: true, tools: [...] }` on success or `{ ok: false, error: ... }` on failure — \
         use this to validate the user's secrets before committing."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": {
                    "type": "string",
                    "description": "Registry qualified name."
                },
                "env_refs": {
                    "type": "object",
                    "description": "Map `{ENV_KEY: \"secret://<hex>\"}` of refs collected from `mcp_setup_request_secret`.",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["qualified_name", "env_refs"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let qualified_name = match read_str(&args, "qualified_name") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let env_refs = match read_str_map(&args, "env_refs") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        outcome_to_result(
            setup_ops::mcp_setup_test_connection(&self.config, qualified_name, env_refs).await,
        )
    }
}

// ── mcp_setup_install_and_connect ────────────────────────────────────────────

pub struct McpSetupInstallAndConnectTool {
    config: Arc<Config>,
}

impl McpSetupInstallAndConnectTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for McpSetupInstallAndConnectTool {
    fn name(&self) -> &str {
        "mcp_setup_install_and_connect"
    }

    fn description(&self) -> &str {
        "Commit: persist the MCP server install + the user's secrets (consuming the refs), then \
         connect immediately. Returns the new `server_id` and (on success) the tool list now \
         available to the agent. Only call after `mcp_setup_test_connection` returned ok."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": {
                    "type": "string",
                    "description": "Registry qualified name."
                },
                "env_refs": {
                    "type": "object",
                    "description": "Same shape as `mcp_setup_test_connection`. Refs are consumed (removed from the in-memory map) on success.",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["qualified_name", "env_refs"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let qualified_name = match read_str(&args, "qualified_name") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let env_refs = match read_str_map(&args, "env_refs") {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        outcome_to_result(
            setup_ops::mcp_setup_install_and_connect(&self.config, qualified_name, env_refs).await,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable() {
        let cfg = Arc::new(Config::default());
        assert_eq!(
            McpSetupSearchTool::new(cfg.clone()).name(),
            "mcp_setup_search"
        );
        assert_eq!(McpSetupGetTool::new(cfg.clone()).name(), "mcp_setup_get");
        assert_eq!(
            McpSetupRequestSecretTool::new().name(),
            "mcp_setup_request_secret"
        );
        assert_eq!(
            McpSetupTestConnectionTool::new(cfg.clone()).name(),
            "mcp_setup_test_connection"
        );
        assert_eq!(
            McpSetupInstallAndConnectTool::new(cfg).name(),
            "mcp_setup_install_and_connect"
        );
    }

    #[test]
    fn read_str_map_rejects_non_string_values() {
        let args = json!({ "env_refs": { "K": 42 } });
        assert!(read_str_map(&args, "env_refs").is_err());
    }
}
