//! LLM-callable wrappers over the `skills` metadata domain.
//!
//! These tools let the agent discover installed skills, inspect a skill's
//! definition and bundled resources, review recent runs and their logs, and
//! (opt-in) scaffold / install / uninstall user skills. Thin shims over the
//! free functions in the `skills::ops_*` / `registry` / `run_log` modules.
//!
//! NOTE: launching a skill run is already exposed by `RunSkillTool`
//! (`skills.run`), so it is intentionally not duplicated here.
//!
//! Read tools are default-enabled. The write/install/uninstall tools
//! (`skill_create`, `skill_install_from_url`, `skill_uninstall`) mutate the
//! on-disk skill set (and install fetches remote content), so they ship
//! default-OFF via `tools/user_filter.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops_create::{create_skill, CreateSkillParams};
use super::ops_discover::{discover_skills, is_workspace_trusted, read_skill_resource};
use super::ops_install::{
    install_skill_from_url, uninstall_skill, InstallSkillFromUrlParams, UninstallSkillParams,
};
use super::registry::get_skill;
use super::run_log::{find_run_log_path, read_run_log_slice, scan_runs};

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// List installed skills.
pub struct SkillListTool {
    workspace_dir: PathBuf,
}

impl SkillListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List installed skills (reusable, packaged agent procedures defined as \
         SKILL.md bundles). Returns each skill's name, dir, description, tags, \
         tool hints, scope, and any warnings. Use to find a skill to inspect \
         (`skill_describe`) or run."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] list invoked");
        let home = dirs::home_dir();
        let trusted = is_workspace_trusted(&self.workspace_dir);
        let skills = discover_skills(home.as_deref(), Some(&self.workspace_dir), trusted);
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "count": skills.len(),
            "skills": skills,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Describe one skill (definition + declared inputs).
pub struct SkillDescribeTool {
    workspace_dir: PathBuf,
}

impl SkillDescribeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillDescribeTool {
    fn name(&self) -> &str {
        "skill_describe"
    }

    fn description(&self) -> &str {
        "Describe one skill by `skill_id`: its agent definition (id, \
         display name, when-to-use) and the inputs it declares (name, \
         description, required, type). Use before running a skill to learn \
         which inputs to supply."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "skill_id": { "type": "string", "description": "Skill id (directory name)." } },
            "required": ["skill_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] describe invoked");
        let skill_id = read_required_str(&args, "skill_id")?;
        let def = get_skill(&self.workspace_dir, &skill_id)
            .ok_or_else(|| anyhow::anyhow!("skill_describe: skill `{skill_id}` not found"))?;
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
pub struct SkillReadResourceTool {
    workspace_dir: PathBuf,
}

impl SkillReadResourceTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillReadResourceTool {
    fn name(&self) -> &str {
        "skill_read_resource"
    }

    fn description(&self) -> &str {
        "Read a bundled resource file from a skill (`skill_id` + `relative_path` \
         under the skill directory, e.g. `scripts/run.sh` or \
         `references/spec.md`). Path-hardened and size-capped. Use to inspect a \
         skill's helper scripts or reference docs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string", "description": "Skill id (directory name)." },
                "relative_path": { "type": "string", "description": "Path relative to the skill directory." }
            },
            "required": ["skill_id", "relative_path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] read_resource invoked");
        let skill_id = read_required_str(&args, "skill_id")?;
        let relative_path = read_required_str(&args, "relative_path")?;
        let content =
            read_skill_resource(&self.workspace_dir, &skill_id, Path::new(&relative_path))
                .map_err(|e| anyhow::anyhow!("skill_read_resource: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "skill_id": skill_id,
            "relative_path": relative_path,
            "content": content,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List recent skill runs.
pub struct SkillRecentRunsTool {
    workspace_dir: PathBuf,
}

impl SkillRecentRunsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillRecentRunsTool {
    fn name(&self) -> &str {
        "skill_recent_runs"
    }

    fn description(&self) -> &str {
        "List recent skill runs (optionally filtered by `skill_id`), newest \
         first. Each carries `run_id`, `skill_id`, start time, status, and \
         duration. Use to find a `run_id` for `skill_read_run_log`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string", "description": "Filter to one skill (optional)." },
                "limit": { "type": "integer", "minimum": 1, "description": "Max runs (default 20)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] recent_runs invoked");
        let skill_id = args
            .get("skill_id")
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
pub struct SkillReadRunLogTool {
    workspace_dir: PathBuf,
}

impl SkillReadRunLogTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillReadRunLogTool {
    fn name(&self) -> &str {
        "skill_read_run_log"
    }

    fn description(&self) -> &str {
        "Read a slice of a skill run's log by `run_id`, from `offset` bytes up \
         to `max_bytes`. Returns the content plus the next offset and an `eof` \
         flag so you can stream a long log. Use `skill_recent_runs` to find a \
         run id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Skill run id." },
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
            .ok_or_else(|| anyhow::anyhow!("skill_read_run_log: run `{run_id}` not found"))?;
        let slice = read_run_log_slice(&path, offset, max_bytes)
            .map_err(|e| anyhow::anyhow!("skill_read_run_log: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&slice)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Scaffold a new user skill. **Writes to disk** — default-OFF.
pub struct SkillCreateTool {
    workspace_dir: PathBuf,
}

impl SkillCreateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillCreateTool {
    fn name(&self) -> &str {
        "skill_create"
    }

    fn description(&self) -> &str {
        "Scaffold a new skill (SKILL.md, plus skill.toml when inputs are \
         declared). Requires `name` and `description`; optional `scope` \
         (user|project), `tags`, `allowed_tools`, and `inputs`. Use when the \
         user wants to capture a repeatable procedure as a packaged skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill name (required)." },
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
        let params: CreateSkillParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("skill_create: invalid params: {e}"))?;
        let skill = create_skill(&self.workspace_dir, params)
            .map_err(|e| anyhow::anyhow!("skill_create: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&skill)?))
    }
}

/// Install a skill from a remote URL. **Fetches + writes** — default-OFF.
pub struct SkillInstallFromUrlTool {
    workspace_dir: PathBuf,
}

impl SkillInstallFromUrlTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillInstallFromUrlTool {
    fn name(&self) -> &str {
        "skill_install_from_url"
    }

    fn description(&self) -> &str {
        "Install a user skill from a remote `url` (https, must point at a \
         SKILL.md). Fetches and writes it under `~/.openhuman/skills/`. \
         Optional `timeout_secs`. Collisions are rejected. Only use when the \
         user explicitly asks to install a skill from a URL."
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
        let params: InstallSkillFromUrlParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("skill_install_from_url: invalid params: {e}"))?;
        let outcome = install_skill_from_url(&self.workspace_dir, params)
            .await
            .map_err(|e| anyhow::anyhow!("skill_install_from_url: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome)?))
    }
}

/// Uninstall a user skill. **Deletes from disk** — default-OFF.
pub struct SkillUninstallTool;

#[async_trait]
impl Tool for SkillUninstallTool {
    fn name(&self) -> &str {
        "skill_uninstall"
    }

    fn description(&self) -> &str {
        "Uninstall a user-scope skill by `name`, deleting its directory under \
         `~/.openhuman/skills/`. Irreversible; project/legacy skills are \
         read-only and cannot be removed. Only use when the user asks to remove \
         a specific skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "Skill name (directory) to remove." } },
            "required": ["name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][skills] uninstall invoked");
        let name = read_required_str(&args, "name")?;
        let outcome = uninstall_skill(UninstallSkillParams { name }, None)
            .map_err(|e| anyhow::anyhow!("skill_uninstall: {e}"))?;
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
    fn names_and_levels() {
        let c = cfg();
        assert_eq!(SkillListTool::new(c.clone()).name(), "skill_list");
        assert_eq!(
            SkillListTool::new(c.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            SkillCreateTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            SkillInstallFromUrlTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert!(SkillInstallFromUrlTool::new(c.clone())
            .external_effect_with_args(&serde_json::Value::Null));
        assert_eq!(
            SkillUninstallTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(SkillListTool::new(c).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn describe_requires_skill_id() {
        let err = SkillDescribeTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing skill_id");
        assert!(err.to_string().contains("skill_id"));
    }

    #[tokio::test]
    async fn read_resource_requires_both_args() {
        let err = SkillReadResourceTool::new(cfg())
            .execute(json!({ "skill_id": "x" }))
            .await
            .expect_err("missing relative_path");
        assert!(err.to_string().contains("relative_path"));
    }

    #[tokio::test]
    async fn uninstall_requires_name() {
        let err = SkillUninstallTool
            .execute(json!({}))
            .await
            .expect_err("missing name");
        assert!(err.to_string().contains("name"));
    }

    #[tokio::test]
    async fn list_returns_envelope() {
        // A fresh workspace has no project skills, but the user-home scan may
        // surface bundled skills; either way the call succeeds and returns the
        // envelope shape.
        let out = SkillListTool::new(cfg())
            .execute(json!({}))
            .await
            .expect("list");
        assert!(out.output_for_llm(false).contains("skills"));
    }
}
