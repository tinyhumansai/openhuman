//! LLM-callable wrappers over the `workflows` metadata domain.
//!
//! These tools let the agent discover installed workflows, inspect a
//! workflow's definition and bundled resources, review recent runs and their
//! logs, and (opt-in) scaffold / install / uninstall user workflows. Thin
//! shims over the free functions in the `workflows::ops_*` / `registry` /
//! `run_log` modules.
//!
//! NOTE: launching a workflow is exposed separately by `RunWorkflowTool`
//! (`run_workflow`) + `AwaitWorkflowTool`, so it is not duplicated here.
//!
//! Read tools are default-enabled. The write/install/uninstall tools
//! (`create_workflow`, `install_workflow_from_url`, `uninstall_workflow`)
//! mutate the on-disk workflow set (and install fetches remote content), so
//! they ship default-OFF via `tools/user_filter.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops_create::{create_workflow, CreateWorkflowParams};
use super::ops_discover::{discover_workflows, is_workspace_trusted, read_workflow_resource};
use super::ops_install::{
    install_workflow_from_url, uninstall_workflow, InstallWorkflowFromUrlParams,
    UninstallWorkflowParams,
};
use super::registry::get_workflow;
use super::run_log::{find_run_log_path, read_run_log_slice, scan_runs};

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// Read the target workflow id, accepting the legacy `skill_id` key as an
/// alias for `workflow_id` so callers from before the rename still work.
fn read_workflow_id(args: &serde_json::Value) -> anyhow::Result<String> {
    read_required_str(args, "workflow_id")
        .or_else(|_| read_required_str(args, "skill_id"))
        .map_err(|_| anyhow::anyhow!("missing required string argument `workflow_id`"))
}

/// Skill/workflow allowlist applied per agent profile. `None` = all skills are
/// visible (the default). `Some(set)` restricts to the named `dir_name` slugs.
type SkillAllowlist = Option<std::collections::HashSet<String>>;

/// Whether `dir_name` passes the optional per-profile skill allowlist.
fn skill_allowed(allowlist: &SkillAllowlist, dir_name: &str) -> bool {
    match allowlist {
        None => true,
        Some(set) => set.contains(dir_name),
    }
}

/// List installed skills.
pub struct WorkflowListTool {
    workspace_dir: PathBuf,
    skill_allowlist: SkillAllowlist,
}

impl WorkflowListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
            skill_allowlist: None,
        }
    }

    /// Scope the listed workflows to a per-profile allowlist of `dir_name`
    /// slugs. `None` leaves all workflows visible.
    pub fn with_skill_allowlist(mut self, allowlist: SkillAllowlist) -> Self {
        self.skill_allowlist = allowlist;
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
        let home = dirs::home_dir();
        let trusted = is_workspace_trusted(&self.workspace_dir);
        let mut workflows = discover_workflows(home.as_deref(), Some(&self.workspace_dir), trusted);
        if self.skill_allowlist.is_some() {
            let before = workflows.len();
            workflows.retain(|w| skill_allowed(&self.skill_allowlist, &w.dir_name));
            log::debug!(
                "[profiles] list_workflows scoped to profile allowlist: before={before} after={}",
                workflows.len()
            );
        }
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
    skill_allowlist: SkillAllowlist,
}

impl WorkflowDescribeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
            skill_allowlist: None,
        }
    }

    /// Scope describe access to a per-profile allowlist of `dir_name` slugs.
    pub fn with_skill_allowlist(mut self, allowlist: SkillAllowlist) -> Self {
        self.skill_allowlist = allowlist;
        self
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
        if !skill_allowed(&self.skill_allowlist, &skill_id) {
            log::debug!("[profiles] describe_workflow blocked by profile allowlist: {skill_id}");
            return Ok(ToolResult::error(format!(
                "describe_workflow: workflow `{skill_id}` is not available to the active agent profile"
            )));
        }
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
    workspace_dir: PathBuf,
    skill_allowlist: SkillAllowlist,
}

impl WorkflowReadResourceTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
            skill_allowlist: None,
        }
    }

    /// Scope resource reads to a per-profile allowlist of `dir_name` slugs, so a
    /// restricted profile can't exfiltrate the bundled scripts/docs of a
    /// workflow outside its skill set.
    pub fn with_skill_allowlist(mut self, allowlist: SkillAllowlist) -> Self {
        self.skill_allowlist = allowlist;
        self
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
        if !skill_allowed(&self.skill_allowlist, &skill_id) {
            log::debug!(
                "[profiles] read_workflow_resource blocked by profile allowlist: {skill_id}"
            );
            return Ok(ToolResult::error(format!(
                "read_workflow_resource: workflow `{skill_id}` is not available to the active agent profile"
            )));
        }
        let relative_path = read_required_str(&args, "relative_path")?;
        let content =
            read_workflow_resource(&self.workspace_dir, &skill_id, Path::new(&relative_path))
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
        let runs = scan_runs(&self.workspace_dir, skill_id, limit);
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
        let path = find_run_log_path(&self.workspace_dir, &run_id)
            .ok_or_else(|| anyhow::anyhow!("read_workflow_run_log: run `{run_id}` not found"))?;
        let slice = read_run_log_slice(&path, offset, max_bytes)
            .map_err(|e| anyhow::anyhow!("read_workflow_run_log: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&slice)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

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
        "create_workflow"
    }

    fn description(&self) -> &str {
        "Scaffold a new workflow (SKILL.md, plus skill.toml when inputs are \
         declared). Requires `name` and `description`; optional `scope` \
         (user|project), `tags`, `allowed_tools`, and `inputs`. Use when the \
         user wants to capture a repeatable procedure as a packaged workflow."
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
            .map_err(|e| anyhow::anyhow!("create_workflow: invalid params: {e}"))?;
        let skill = create_workflow(&self.workspace_dir, params)
            .map_err(|e| anyhow::anyhow!("create_workflow: {e}"))?;
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
         Optional `timeout_secs`. Collisions are rejected. Only use when the \
         user explicitly asks to install a workflow from a URL."
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
        let outcome = install_workflow_from_url(&self.workspace_dir, params)
            .await
            .map_err(|e| anyhow::anyhow!("install_workflow_from_url: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome)?))
    }
}

/// Uninstall a user skill. **Deletes from disk** — default-OFF.
pub struct WorkflowUninstallTool;

#[async_trait]
impl Tool for WorkflowUninstallTool {
    fn name(&self) -> &str {
        "uninstall_workflow"
    }

    fn description(&self) -> &str {
        "Uninstall a user-scope workflow by `name`, deleting its directory under \
         `~/.openhuman/skills/`. Irreversible; project/legacy workflows are \
         read-only and cannot be removed. Only use when the user asks to remove \
         a specific workflow."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "Workflow name (directory) to remove." } },
            "required": ["name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] uninstall invoked");
        let name = read_required_str(&args, "name")?;
        let outcome = uninstall_workflow(UninstallWorkflowParams { name }, None)
            .map_err(|e| anyhow::anyhow!("uninstall_workflow: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome)?))
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
    fn skill_allowed_respects_optional_allowlist() {
        // None = all skills visible.
        assert!(skill_allowed(&None, "deep-research"));
        // Some(set) restricts to named dir_name slugs.
        let set: std::collections::HashSet<String> =
            ["deep-research".to_string()].into_iter().collect();
        assert!(skill_allowed(&Some(set.clone()), "deep-research"));
        assert!(!skill_allowed(&Some(set), "ship-and-babysit"));
        // Empty allowlist blocks everything (profile selected no skills).
        assert!(!skill_allowed(
            &Some(std::collections::HashSet::new()),
            "anything"
        ));
    }

    #[tokio::test]
    async fn describe_workflow_blocks_disallowed_skill_before_lookup() {
        let allow: std::collections::HashSet<String> =
            ["allowed-skill".to_string()].into_iter().collect();
        let tool = WorkflowDescribeTool::new(cfg()).with_skill_allowlist(Some(allow));
        let res = tool
            .execute(json!({ "workflow_id": "blocked-skill" }))
            .await
            .expect("execute");
        assert!(res.is_error, "disallowed skill must return an error result");
        let text = serde_json::to_string(&res.content).expect("serialize content");
        assert!(
            text.contains("not available to the active agent profile"),
            "expected profile-allowlist rejection, got: {text}"
        );
    }

    #[test]
    fn names_and_levels() {
        let c = cfg();
        assert_eq!(WorkflowListTool::new(c.clone()).name(), "list_workflows");
        assert_eq!(
            WorkflowListTool::new(c.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            WorkflowCreateTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            WorkflowInstallFromUrlTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert!(WorkflowInstallFromUrlTool::new(c.clone())
            .external_effect_with_args(&serde_json::Value::Null));
        assert_eq!(
            WorkflowUninstallTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(WorkflowListTool::new(c).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn describe_requires_workflow_id() {
        let err = WorkflowDescribeTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing workflow_id");
        assert!(err.to_string().contains("workflow_id"));
    }

    #[tokio::test]
    async fn describe_accepts_legacy_skill_id_alias() {
        // `skill_id` still resolves (back-compat) — a non-existent id should
        // fail with "not found", not "missing argument".
        let err = WorkflowDescribeTool::new(cfg())
            .execute(json!({ "skill_id": "does-not-exist" }))
            .await
            .expect_err("unknown workflow");
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn read_resource_requires_both_args() {
        let err = WorkflowReadResourceTool::new(cfg())
            .execute(json!({ "workflow_id": "x" }))
            .await
            .expect_err("missing relative_path");
        assert!(err.to_string().contains("relative_path"));
    }

    #[tokio::test]
    async fn uninstall_requires_name() {
        let err = WorkflowUninstallTool
            .execute(json!({}))
            .await
            .expect_err("missing name");
        assert!(err.to_string().contains("name"));
    }

    #[tokio::test]
    async fn list_returns_envelope() {
        // A fresh workspace has no project workflows, but the user-home scan
        // may surface bundled ones; either way the call succeeds and returns
        // the envelope shape.
        let out = WorkflowListTool::new(cfg())
            .execute(json!({}))
            .await
            .expect("list");
        assert!(out.output_for_llm(false).contains("workflows"));
    }
}
