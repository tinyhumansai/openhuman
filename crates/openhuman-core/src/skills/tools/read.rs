//! Read-only workflow tools: list, describe, read a bundled resource, list
//! recent runs, and read a run log.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::config::Config;
use tinytools::{Tool, ToolResult};

use super::super::ops_discover::read_workflow_resource_for;
use super::super::registry::get_workflow;
use super::super::run_log::{read_run_log_slice, scan_runs};
use super::helpers::{read_required_str, read_workflow_id};

/// List installed skills.
pub struct WorkflowListTool {
    config: Arc<Config>,
    /// User-scope root to scan, when it must not be the real `$HOME`.
    ///
    /// `None` — production — resolves `dirs::home_dir()` at call time, exactly
    /// as before. See [`Self::with_home_dir`].
    home_dir: Option<PathBuf>,
}

impl WorkflowListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            home_dir: None,
        }
    }

    /// Scan `home` as the user scope instead of `dirs::home_dir()`.
    ///
    /// Every other layer of discovery already takes the home directory as a
    /// parameter — `discover_workflows_inner` and
    /// `install_workflow_from_url_with_home` both do — and this tool was the
    /// one place that resolved it internally, which made it the one place a test
    /// could not isolate.
    ///
    /// That is not a hypothetical: a test seeding one project-scope workflow
    /// into a tempdir also picked up every skill the developer had installed
    /// under their real `~/.openhuman/skills` and `~/.agents/skills`. The result
    /// is one JSON blob, the harness caps a tool result at 16 KiB
    /// (`ContextConfig::tool_result_budget_bytes`), and fifteen real bundles
    /// push past that — so the seeded workflow was discovered correctly and then
    /// truncated back out before the assertion could see it. The failure looked
    /// like broken discovery and was a non-hermetic fixture.
    #[must_use]
    pub fn with_home_dir(mut self, home: Option<PathBuf>) -> Self {
        self.home_dir = home;
        self
    }
}

#[async_trait]
impl Tool for WorkflowListTool {
    fn name(&self) -> &str {
        "list_workflows"
    }

    fn description(&self) -> &str {
        "List installed workflows (reusable, packaged agent procedures — a goal \
         plus the procedure to reach it). Returns each workflow's name, dir, \
         description, tags, tool hints, scope, and any warnings. Use to find a \
         workflow to inspect (`describe_workflow`) or run (`run_workflow`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workflows] list invoked");
        let ctx = crate::core::runtime::DispatchCtx::ambient();
        let workflows = crate::skills::ops_discover::load_workflow_metadata_for_with_home(
            &ctx,
            &self.config,
            self.home_dir.clone(),
        );
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "count": workflows.len(),
            "workflows": workflows,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Describe one skill (definition + declared inputs).
pub struct WorkflowDescribeTool {
    workspace_dir: PathBuf,
}

impl WorkflowDescribeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for WorkflowDescribeTool {
    fn name(&self) -> &str {
        "describe_workflow"
    }

    fn description(&self) -> &str {
        "Describe one workflow by `workflow_id`: its agent definition (id, \
         display name, when-to-use) and the inputs it declares (name, \
         description, required, type). Use before running a workflow to learn \
         which inputs to supply."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "workflow_id": { "type": "string", "description": "Workflow id (directory name)." } },
            "required": ["workflow_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workflows] describe invoked");
        let skill_id = read_workflow_id(&args)?;
        let def = get_workflow(&self.workspace_dir, &skill_id)
            .ok_or_else(|| anyhow::anyhow!("describe_workflow: workflow `{skill_id}` not found"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "definition": def.definition,
            "inputs": def.inputs,
            "github_gated": def.github.is_some(),
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read a bundled resource file from a skill.
pub struct WorkflowReadResourceTool {
    config: Arc<Config>,
}

impl WorkflowReadResourceTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WorkflowReadResourceTool {
    fn name(&self) -> &str {
        "read_workflow_resource"
    }

    fn description(&self) -> &str {
        "Read a bundled resource file from a workflow (`workflow_id` + \
         `relative_path` under the workflow directory, e.g. `scripts/run.sh` or \
         `references/spec.md`). Path-hardened and size-capped. Use to inspect a \
         workflow's helper scripts or reference docs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "workflow_id": { "type": "string", "description": "Workflow id (directory name)." },
                "relative_path": { "type": "string", "description": "Path relative to the workflow directory." }
            },
            "required": ["workflow_id", "relative_path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workflows] read_resource invoked");
        let skill_id = read_workflow_id(&args)?;
        let relative_path = read_required_str(&args, "relative_path")?;
        let ctx = crate::core::runtime::DispatchCtx::ambient();
        let content =
            read_workflow_resource_for(&ctx, &self.config, &skill_id, Path::new(&relative_path))
                .map_err(|e| anyhow::anyhow!("read_workflow_resource: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "workflow_id": skill_id,
            "relative_path": relative_path,
            "content": content,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List recent skill runs.
pub struct WorkflowRecentRunsTool {
    workspace_dir: PathBuf,
}

impl WorkflowRecentRunsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for WorkflowRecentRunsTool {
    fn name(&self) -> &str {
        "list_workflow_runs"
    }

    fn description(&self) -> &str {
        "List recent workflow runs (optionally filtered by `workflow_id`), \
         newest first. Each carries `run_id`, `workflow_id`, start time, status, \
         and duration. Use to find a `run_id` for `read_workflow_run_log` or \
         `await_workflow`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "workflow_id": { "type": "string", "description": "Filter to one workflow (optional)." },
                "limit": { "type": "integer", "minimum": 1, "description": "Max runs (default 20)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][workflows] recent_runs invoked");
        let skill_id = args
            .get("workflow_id")
            .or_else(|| args.get("skill_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(20);
        let runs = scan_runs(&self.workspace_dir, skill_id, usize::MAX)
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "count": runs.len(),
            "runs": runs,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read a slice of a run log.
pub struct WorkflowReadRunLogTool {
    workspace_dir: PathBuf,
}

impl WorkflowReadRunLogTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for WorkflowReadRunLogTool {
    fn name(&self) -> &str {
        "read_workflow_run_log"
    }

    fn description(&self) -> &str {
        "Read a slice of a workflow run's log by `run_id`, from `offset` bytes \
         up to `max_bytes`. Returns the content plus the next offset and an \
         `eof` flag so you can stream a long log. Use `list_workflow_runs` to \
         find a run id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Workflow run id." },
                "offset": { "type": "integer", "minimum": 0, "description": "Byte offset to start at (default 0)." },
                "max_bytes": { "type": "integer", "minimum": 1, "description": "Max bytes to read (default 65536)." }
            },
            "required": ["run_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] read_run_log invoked");
        let run_id = read_required_str(&args, "run_id")?;
        let offset = args
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let max_bytes = args
            .get("max_bytes")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(65536);
        let run = scan_runs(&self.workspace_dir, None, usize::MAX)
            .into_iter()
            .find(|run| run.run_id == run_id)
            .ok_or_else(|| anyhow::anyhow!("read_workflow_run_log: run `{run_id}` not found"))?;
        let path = PathBuf::from(run.log_path);
        let slice = read_run_log_slice(&self.workspace_dir, &path, offset, max_bytes)
            .map_err(|e| anyhow::anyhow!("read_workflow_run_log: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&slice)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}
