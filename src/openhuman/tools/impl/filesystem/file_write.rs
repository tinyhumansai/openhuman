use crate::openhuman::agent::file_state;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::ToolRunContext;

const MAX_CONTENT_BYTES: usize = 5 * 1024 * 1024;

/// Write file contents with path sandboxing
pub struct FileWriteTool {
    security: Arc<SecurityPolicy>,
    approval_workspace_root: Option<std::path::PathBuf>,
    sink: Arc<dyn super::write_sink::FileSink>,
}

impl FileWriteTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self {
            security,
            approval_workspace_root: None,
            sink: super::write_sink::os_sink(),
        }
    }

    /// Sends this tool's writes somewhere other than the OS.
    #[cfg(test)]
    pub fn with_sink(mut self, sink: Arc<dyn super::write_sink::FileSink>) -> Self {
        self.sink = sink;
        self
    }

    /// Use the same effective workspace root for approval routing that tool
    /// execution receives through its [`ToolExecutionContext`].
    pub fn with_approval_workspace_root(
        security: Arc<SecurityPolicy>,
        approval_workspace_root: std::path::PathBuf,
    ) -> Self {
        Self {
            security,
            approval_workspace_root: Some(approval_workspace_root),
            sink: super::write_sink::os_sink(),
        }
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write contents to a file in your working directory (the action sandbox). \
         Relative paths resolve against that directory; writes outside it are blocked. \
         Reference the file later by the same relative path so `file_read` resolves to it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the file within the workspace"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    /// "Ask before edit": modifying an **existing** file routes through the
    /// human approval gate in ask-before-edit mode; creating a **new** file is
    /// free. In Full neither prompts; in read-only `execute` blocks via
    /// `can_act()`. The existence probe is best-effort (relative to the
    /// workspace); when the path can't be resolved we fail safe and prompt.
    ///
    /// The probe runs at gate-routing time, microseconds before `execute()` in
    /// the same sequential turn. A create→edit flip in that window would require
    /// an external process to win the race — outside this gate's threat model,
    /// which governs the agent, not concurrent writers — and `execute()` still
    /// enforces workspace containment + symlink refusal regardless, so a write
    /// that slips through as "create" cannot escape the sandbox. We therefore
    /// do not re-probe in `execute()`.
    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        if self.security.gate_decision(CommandClass::Write) != GateDecision::Prompt {
            return false; // Full (allow) or read-only (blocked in execute)
        }
        let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
            return true; // unknown path → prompt (fail safe)
        };
        let target = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            self.approval_workspace_root
                .as_deref()
                .unwrap_or(&self.security.action_dir)
                .join(path)
        };
        // Sync `stat` — intentionally blocking, since the `Tool` trait makes
        // this method sync. Fast for local paths; would only need
        // `block_in_place` if a remote/slow filesystem is ever supported here.
        target.exists() // exists = edit → prompt; new = create → free
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

impl FileWriteTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }

        if content.len() > MAX_CONTENT_BYTES {
            return Ok(ToolResult::error(format!(
                "Content too large: {} bytes (limit: {MAX_CONTENT_BYTES} bytes)",
                content.len()
            )));
        }

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        // Security check first: validate path string, resolve symlinks, confirm workspace
        // containment. validate_parent_path walks up to the deepest existing ancestor so
        // it does not require the parent directory to exist yet.
        let path_policy = super::security_for_tool_context(&self.security, context, "file_write");
        let resolved_target = match path_policy.validate_parent_path(path).await {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        // Create parent directory only at the validated, resolved location.
        if let Some(resolved_parent) = resolved_target.parent() {
            tokio::fs::create_dir_all(resolved_parent).await?;
        }

        // If the target already exists and is a symlink, refuse to follow it
        if let Ok(meta) = tokio::fs::symlink_metadata(&resolved_target).await {
            if meta.file_type().is_symlink() {
                return Ok(ToolResult::error(format!(
                    "Refusing to write through symlink: {}",
                    resolved_target.display()
                )));
            }
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // File-state guard: reject writes based on stale or partial reads.
        if let Some(agent_id) = file_state::current_file_state_agent_id() {
            if let Some(msg) = file_state::check_stale_read(&agent_id, &resolved_target) {
                tracing::debug!(
                    agent = %agent_id,
                    path = %resolved_target.display(),
                    "[file_state] file_write blocked: stale read"
                );
                return Ok(ToolResult::error(msg));
            }
            if let Some(msg) = file_state::check_partial_read(&agent_id, &resolved_target) {
                tracing::debug!(
                    agent = %agent_id,
                    path = %resolved_target.display(),
                    "[file_state] file_write blocked: partial read"
                );
                return Ok(ToolResult::error(msg));
            }
        }

        match self.sink.write(&resolved_target, content.as_bytes()).await {
            Ok(()) => {
                if let Some(agent_id) = file_state::current_file_state_agent_id() {
                    file_state::record_write(&agent_id, resolved_target);
                }
                Ok(ToolResult::success(format!(
                    "Written {} bytes to {path}",
                    content.len()
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "file_write_tests.rs"]
mod tests;
