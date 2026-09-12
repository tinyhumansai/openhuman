//! `list` — directory listing.
//!
//! Coding-harness baseline tool (issue #1205): non-recursive directory
//! listing keyed by a workspace-relative path. Distinguishes files,
//! directories, and symlinks. Path sandboxing matches `file_read`.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::ToolRunContext;

const MAX_ENTRIES: usize = 1_000;

pub struct ListFilesTool {
    security: Arc<SecurityPolicy>,
}

impl ListFilesTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list"
    }

    fn description(&self) -> &str {
        "List entries in a workspace directory (non-recursive). Each line is \
         `<kind>\t<name>` where kind is `dir`, `file`, or `link`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path relative to the workspace (default `.`).",
                    "default": "."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, context).await
    }
}

impl ListFilesTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // Security check: validate path string, resolve symlinks, confirm workspace containment.
        let path_policy = super::security_for_tool_context(&self.security, context, "list");
        let resolved = match path_policy.validate_path(path).await {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        let mut read = match tokio::fs::read_dir(&resolved).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read directory: {e}"))),
        };

        let mut entries: Vec<(String, String)> = Vec::new();
        loop {
            match read.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let kind = match entry.file_type().await {
                        Ok(t) if t.is_symlink() => "link",
                        Ok(t) if t.is_dir() => "dir",
                        Ok(_) => "file",
                        Err(_) => "unknown",
                    };
                    entries.push((kind.to_string(), name));
                    if entries.len() >= MAX_ENTRIES {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    return Ok(ToolResult::error(format!(
                        "Failed to enumerate directory: {e}"
                    )));
                }
            }
        }

        entries.sort_by(|a, b| a.1.cmp(&b.1));

        let mut body = format!("{} entr(ies) in {path}", entries.len());
        for (kind, name) in entries {
            body.push('\n');
            body.push_str(&kind);
            body.push('\t');
            body.push_str(&name);
        }
        Ok(ToolResult::success(body))
    }
}

#[cfg(test)]
#[path = "list_files_tests.rs"]
mod tests;
