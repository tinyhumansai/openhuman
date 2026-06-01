//! LLM-callable wrappers over the `workspace` domain (persona files).
//!
//! `workspace_read_persona` reads an allowlisted persona file (SOUL.md /
//! IDENTITY.md), falling back to the bundled default. It is default-ON.
//!
//! The mutators — rewriting a persona file, resetting it to the bundled
//! default, and scaffolding the workspace — change the assistant's durable
//! identity/scaffold, so they ship default-OFF via `tools/user_filter.rs`
//! (`workspace_manage` toggle).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::workspace::{ops, rpc};

fn req_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// Read an allowlisted persona file.
pub struct WorkspaceReadPersonaTool {
    config: Arc<Config>,
}
impl WorkspaceReadPersonaTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for WorkspaceReadPersonaTool {
    fn name(&self) -> &str {
        "workspace_read_persona"
    }
    fn description(&self) -> &str {
        "Read an allowlisted workspace persona file (`filename` = SOUL.md or \
         IDENTITY.md), returning its contents and whether it is the bundled \
         default."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "filename": { "type": "string", "enum": ["SOUL.md", "IDENTITY.md"] } },
            "required": ["filename"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workspace] read_persona invoked");
        let filename = req_str(&args, "filename")?;
        let outcome = rpc::read_workspace_file(&self.config.workspace_dir, &filename)
            .map_err(|e| anyhow::anyhow!("workspace_read_persona: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Rewrite a persona file. Default-OFF.
pub struct WorkspaceUpdatePersonaTool {
    config: Arc<Config>,
}
impl WorkspaceUpdatePersonaTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for WorkspaceUpdatePersonaTool {
    fn name(&self) -> &str {
        "workspace_update_persona"
    }
    fn description(&self) -> &str {
        "Overwrite an allowlisted persona file (`filename` = SOUL.md or \
         IDENTITY.md) with new `contents`. Changes the assistant's durable \
         identity. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filename": { "type": "string", "enum": ["SOUL.md", "IDENTITY.md"] },
                "contents": { "type": "string" }
            },
            "required": ["filename", "contents"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workspace] update_persona invoked");
        let filename = req_str(&args, "filename")?;
        let contents = args
            .get("contents")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument `contents`"))?;
        let outcome = rpc::write_workspace_file(&self.config.workspace_dir, &filename, contents)
            .map_err(|e| anyhow::anyhow!("workspace_update_persona: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

/// Reset a persona file to its bundled default. Default-OFF.
pub struct WorkspaceResetPersonaTool {
    config: Arc<Config>,
}
impl WorkspaceResetPersonaTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for WorkspaceResetPersonaTool {
    fn name(&self) -> &str {
        "workspace_reset_persona"
    }
    fn description(&self) -> &str {
        "Reset an allowlisted persona file (`filename`) to its bundled default, \
         overwriting any customization. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "filename": { "type": "string", "enum": ["SOUL.md", "IDENTITY.md"] } },
            "required": ["filename"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workspace] reset_persona invoked");
        let filename = req_str(&args, "filename")?;
        let outcome = rpc::reset_workspace_file(&self.config.workspace_dir, &filename)
            .map_err(|e| anyhow::anyhow!("workspace_reset_persona: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

/// Scaffold the workspace. Default-OFF.
pub struct WorkspaceInitTool;
#[async_trait]
impl Tool for WorkspaceInitTool {
    fn name(&self) -> &str {
        "workspace_init"
    }
    fn description(&self) -> &str {
        "Scaffold the workspace (memory/sessions/state dirs, bundled prompts, \
         HEARTBEAT.md). `force` re-initializes existing files. Default-OFF \
         (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "force": { "type": "boolean", "description": "Re-initialize existing files (default false)." } }
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workspace] init invoked");
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        let value = ops::init_workspace(force)
            .await
            .map_err(|e| anyhow::anyhow!("workspace_init: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&value)?))
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
            WorkspaceReadPersonaTool::new(cfg()).name(),
            "workspace_read_persona"
        );
        assert_eq!(
            WorkspaceReadPersonaTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            WorkspaceUpdatePersonaTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(WorkspaceInitTool.permission_level(), PermissionLevel::Write);
        assert_eq!(WorkspaceReadPersonaTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn read_requires_filename() {
        let err = WorkspaceReadPersonaTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing filename");
        assert!(err.to_string().contains("filename"));
    }
}
