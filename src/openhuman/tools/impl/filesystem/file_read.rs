use crate::openhuman::agent::file_state;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::ToolRunContext;

const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// Read file contents with path sandboxing
pub struct FileReadTool {
    security: Arc<SecurityPolicy>,
}

impl FileReadTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file in your working directory (the action sandbox). \
         Relative paths resolve against that directory; paths outside it are blocked. \
         To read a file written by `shell`, confirm its location with `pwd` and use the \
         same relative path."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the file within the workspace"
                }
            },
            "required": ["path"]
        })
    }

    /// Pure read — safe to fan out across parallel `file_read` calls.
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
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

impl FileReadTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        // Record action BEFORE validation so that every non-trivially-rejected
        // request consumes rate limit budget. This prevents attackers from probing
        // path existence (via canonicalize errors) without rate limit cost.
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // Security check: validate path string, resolve symlinks, confirm workspace containment.
        let path_policy = super::security_for_tool_context(&self.security, context, "file_read");
        let resolved_path = match path_policy.validate_path(path).await {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        // Check file size AFTER canonicalization to prevent TOCTOU symlink bypass
        match tokio::fs::metadata(&resolved_path).await {
            Ok(meta) => {
                if meta.len() > MAX_FILE_SIZE_BYTES {
                    return Ok(ToolResult::error(format!(
                        "File too large: {} bytes (limit: {MAX_FILE_SIZE_BYTES} bytes)",
                        meta.len()
                    )));
                }
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to read file metadata: {e}"
                )));
            }
        }

        match tokio::fs::read_to_string(&resolved_path).await {
            Ok(contents) => {
                if let Some(agent_id) = file_state::current_file_state_agent_id() {
                    let mtime = tokio::fs::metadata(&resolved_path)
                        .await
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    file_state::record_read(&agent_id, resolved_path, mtime, false);
                }
                Ok(ToolResult::success(contents))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to read file: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "file_read_tests.rs"]
mod tests;
