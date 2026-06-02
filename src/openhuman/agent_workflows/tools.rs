//! LLM-callable wrappers over the agent-workflows domain.
//!
//! These tools let the agent discover, read, scaffold, and inspect the
//! lifecycle phases of installed WORKFLOW.md bundles (under
//! `~/.openhuman/workflows/` and the workspace). They are thin shims over
//! the free functions re-exported from
//! [`crate::openhuman::agent_workflows`].
//!
//! `agent_workflow_list` / `read` / `phase_info` are read-only and
//! default-enabled; `agent_workflow_create` is a bounded `Write`
//! (scaffolds a user-scope dir) and default-enabled. `agent_workflow_uninstall`
//! recursively deletes a workflow directory — it is `Dangerous` and ships
//! default-OFF via `tools/user_filter.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::agent_workflows::{
    create_workflow, discover_workflows, effective_tool_scope, is_workspace_trusted,
    phase_guidance, read_workflow, uninstall_workflow,
};
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

fn read_optional_str(args: &serde_json::Value, key: &str) -> String {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// List installed agent workflows (user + project scope).
pub struct AgentWorkflowListTool {
    workspace_dir: PathBuf,
}

impl AgentWorkflowListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for AgentWorkflowListTool {
    fn name(&self) -> &str {
        "agent_workflow_list"
    }

    fn description(&self) -> &str {
        "List installed agent workflows (reusable, phased task playbooks \
         defined as WORKFLOW.md bundles). Returns each workflow's `name`, \
         `description`, `when_to_use`, `tags`, `scope`, and phase names. Use \
         this to find a workflow whose `when_to_use` matches the user's \
         request, then `agent_workflow_read` it for the full body and \
         `agent_workflow_phase_info` to resolve a specific phase's guidance."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][agent_workflows] list invoked");
        let trusted = is_workspace_trusted(&self.workspace_dir);
        let workflows = discover_workflows(None, Some(&self.workspace_dir), trusted);
        let body = serde_json::to_string(&json!({
            "count": workflows.len(),
            "workflows": workflows,
        }))?;
        Ok(ToolResult::success(body))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read a single workflow by id (directory name).
pub struct AgentWorkflowReadTool;

#[async_trait]
impl Tool for AgentWorkflowReadTool {
    fn name(&self) -> &str {
        "agent_workflow_read"
    }

    fn description(&self) -> &str {
        "Read one agent workflow by `id` (its directory name), returning the \
         full parsed WORKFLOW.md: description, when-to-use, tags, tool scope, \
         and the per-phase rules/scripts/context. Call `agent_workflow_list` \
         first to discover ids."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Workflow id (directory name)." }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][agent_workflows] read invoked");
        let id = read_required_str(&args, "id")?;
        let workflow =
            read_workflow(&id).map_err(|e| anyhow::anyhow!("agent_workflow_read: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&workflow)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Resolve a single phase's guidance and effective tool scope.
pub struct AgentWorkflowPhaseInfoTool;

#[async_trait]
impl Tool for AgentWorkflowPhaseInfoTool {
    fn name(&self) -> &str {
        "agent_workflow_phase_info"
    }

    fn description(&self) -> &str {
        "For a given workflow `id` and `phase`, return the rendered phase \
         guidance (rules + context as markdown) and the effective tool scope \
         (allow/deny) the agent should honor while in that phase. Use before \
         executing a workflow phase so you follow its rules and stay within \
         its tool allowlist."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Workflow id (directory name)." },
                "phase": { "type": "string", "description": "Phase name to resolve." }
            },
            "required": ["id", "phase"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][agent_workflows] phase_info invoked");
        let id = read_required_str(&args, "id")?;
        let phase = read_required_str(&args, "phase")?;
        let workflow =
            read_workflow(&id).map_err(|e| anyhow::anyhow!("agent_workflow_phase_info: {e}"))?;
        let guidance = phase_guidance(&workflow, &phase);
        let tool_scope = effective_tool_scope(&workflow, &phase);
        let body = serde_json::to_string(&json!({
            "id": id,
            "phase": phase,
            "guidance": guidance,
            "tool_scope": tool_scope,
        }))?;
        Ok(ToolResult::success(body))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Scaffold a new user-scope workflow.
pub struct AgentWorkflowCreateTool;

#[async_trait]
impl Tool for AgentWorkflowCreateTool {
    fn name(&self) -> &str {
        "agent_workflow_create"
    }

    fn description(&self) -> &str {
        "Scaffold a new user-scope agent workflow: creates \
         `~/.openhuman/workflows/<slug>/WORKFLOW.md` from `name`, with an \
         optional `description` and `when_to_use`. Returns the created \
         workflow. Use when the user wants to capture a repeatable, phased \
         procedure as a reusable workflow."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Human-readable workflow name." },
                "description": { "type": "string", "description": "One-line summary." },
                "when_to_use": { "type": "string", "description": "When the agent should reach for this workflow." }
            },
            "required": ["name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][agent_workflows] create invoked");
        let name = read_required_str(&args, "name")?;
        let description = read_optional_str(&args, "description");
        let when_to_use = read_optional_str(&args, "when_to_use");
        let workflow = create_workflow(&name, &description, &when_to_use)
            .map_err(|e| anyhow::anyhow!("agent_workflow_create: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&workflow)?))
    }
}

/// Permanently delete an installed workflow directory. Default-OFF.
pub struct AgentWorkflowUninstallTool;

#[async_trait]
impl Tool for AgentWorkflowUninstallTool {
    fn name(&self) -> &str {
        "agent_workflow_uninstall"
    }

    fn description(&self) -> &str {
        "Permanently delete an installed agent workflow by `id`, removing its \
         entire directory under `~/.openhuman/workflows/`. This is \
         IRREVERSIBLE. Only use when the user has explicitly asked to remove a \
         specific workflow."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Workflow id (directory name) to delete." }
            },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][agent_workflows] uninstall invoked");
        let id = read_required_str(&args, "id")?;
        let removed = uninstall_workflow(&id)
            .map_err(|e| anyhow::anyhow!("agent_workflow_uninstall: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(
            &json!({ "id": id, "removed": removed }),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    #[test]
    fn metadata_is_stable() {
        let cfg = Arc::new(Config::default());
        assert_eq!(
            AgentWorkflowListTool::new(cfg).name(),
            "agent_workflow_list"
        );
        assert_eq!(AgentWorkflowReadTool.name(), "agent_workflow_read");
        assert_eq!(
            AgentWorkflowPhaseInfoTool.name(),
            "agent_workflow_phase_info"
        );
        assert_eq!(AgentWorkflowCreateTool.name(), "agent_workflow_create");
        assert_eq!(
            AgentWorkflowUninstallTool.name(),
            "agent_workflow_uninstall"
        );
    }

    #[test]
    fn permission_levels_match_risk() {
        assert_eq!(
            AgentWorkflowReadTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            AgentWorkflowCreateTool.permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            AgentWorkflowUninstallTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(AgentWorkflowReadTool.scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn read_requires_id() {
        let err = AgentWorkflowReadTool
            .execute(json!({}))
            .await
            .expect_err("expected missing-arg error");
        assert!(err.to_string().contains("id"));
    }

    #[tokio::test]
    async fn phase_info_requires_id_and_phase() {
        let err = AgentWorkflowPhaseInfoTool
            .execute(json!({ "id": "x" }))
            .await
            .expect_err("expected missing-arg error");
        assert!(err.to_string().contains("phase"));
    }

    #[tokio::test]
    async fn uninstall_requires_id() {
        let err = AgentWorkflowUninstallTool
            .execute(json!({ "id": "" }))
            .await
            .expect_err("expected missing-arg error");
        assert!(err.to_string().contains("id"));
    }
}
