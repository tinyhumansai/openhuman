//! Mutating workflow tools: scaffold and install-from-url. Uninstall lives in
//! `tools_uninstall.rs`. All ship default-OFF via `tools/user_filter.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolResult};

use super::super::ops_create::{create_workflow, CreateWorkflowParams};
use super::super::ops_install::{
    install_workflow_from_url, InstallWorkflowFromUrlParams, ScanAcknowledgement,
    SkillInstallOutcome,
};

/// Scaffold a new user skill. **Writes to disk** — default-OFF.
pub struct WorkflowCreateTool {
    workspace_dir: PathBuf,
}

impl WorkflowCreateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for WorkflowCreateTool {
    fn name(&self) -> &str {
        // Renamed from `create_workflow` (audit F8): "workflow" now
        // unambiguously means a Flows automation — this scaffolds a SKILL.md
        // *skill*. The flows domain owns `create_workflow`.
        "create_skill"
    }

    fn description(&self) -> &str {
        "Scaffold a new SKILL.md skill (a packaged, repeatable procedure), plus skill.toml when \
         inputs are declared. Requires `name` and `description`; optional `scope` (user|project), \
         `tags`, `allowed_tools`, and `inputs`. NOTE: this creates a *skill*, not a Flows \
         automation workflow — use create_workflow for that."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Workflow name (required)." },
                "description": { "type": "string", "description": "One-line summary (required)." },
                "scope": { "type": "string", "enum": ["user", "project"], "description": "Install scope (default user)." },
                "license": { "type": "string" },
                "author": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "allowed_tools": { "type": "array", "items": { "type": "string" } },
                "inputs": { "type": "array", "items": { "type": "object" } }
            },
            "required": ["name", "description"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] create invoked");
        let params: CreateWorkflowParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("create_skill: invalid params: {e}"))?;
        let skill = create_workflow(&self.workspace_dir, params)
            .map_err(|e| anyhow::anyhow!("create_skill: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&skill)?))
    }
}

/// Install a skill from a remote URL. **Fetches + writes** — default-OFF.
pub struct WorkflowInstallFromUrlTool {
    workspace_dir: PathBuf,
}

impl WorkflowInstallFromUrlTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for WorkflowInstallFromUrlTool {
    fn name(&self) -> &str {
        "install_workflow_from_url"
    }

    fn description(&self) -> &str {
        "Install a user workflow from a remote `url` (https, must point at a \
         SKILL.md). Fetches and writes it under `~/.openhuman/skills/`. \
         Optional `timeout_secs`. Collisions are rejected. A `scan_blocked` \
         result means the security scan refused it and nothing was installed. \
         Only use when the user explicitly asks to install a workflow from a URL."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "https URL ending in .md (required)." },
                "timeout_secs": { "type": "integer", "minimum": 1, "description": "Fetch timeout (default 60, max 600)." }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect(&self) -> bool {
        // Fetches remote content over the network.
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] install_from_url invoked");
        let params: InstallWorkflowFromUrlParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("install_workflow_from_url: invalid params: {e}"))?;
        let outcome =
            install_workflow_from_url(&self.workspace_dir, params, ScanAcknowledgement::Absent)
                .await
                .map_err(|e| anyhow::anyhow!("install_workflow_from_url: {e}"))?;
        match outcome {
            SkillInstallOutcome::Installed(installed) => {
                Ok(ToolResult::success(serde_json::to_string(&installed)?))
            }
            SkillInstallOutcome::ScanBlocked(blocked) => {
                crate::skills::catalog::tools::scan_blocked_tool_result(&blocked)
            }
        }
    }
}
