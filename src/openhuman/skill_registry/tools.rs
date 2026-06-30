//! LLM-callable tools for the skill registry domain.
//!
//! These tools let the orchestrator (and other agents) browse the aggregated
//! Hermes catalog, search for skills, and install from catalog entries.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;

pub struct SkillRegistryBrowseTool;

#[async_trait]
impl Tool for SkillRegistryBrowseTool {
    fn name(&self) -> &str {
        "skill_registry_browse"
    }

    fn description(&self) -> &str {
        "Browse the aggregated skill catalog (HermesHub, ClawHub, skills.sh, \
         LobeHub, browse.sh). Returns all available skills with metadata. \
         Use `force_refresh: true` to bypass the cache."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "force_refresh": {
                    "type": "boolean",
                    "description": "Force re-fetch from the Hermes API, bypassing cache.",
                    "default": false
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let force = args
            .get("force_refresh")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        tracing::debug!(force_refresh = force, "[tool][skill_registry] browse");

        match ops::browse_catalog(force).await {
            Ok(entries) => Ok(ToolResult::success(serde_json::to_string(&json!({
                "count": entries.len(),
                "entries": entries,
            }))?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to browse skill catalog: {e}"
            ))),
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

pub struct SkillRegistrySearchTool;

#[async_trait]
impl Tool for SkillRegistrySearchTool {
    fn name(&self) -> &str {
        "skill_registry_search"
    }

    fn description(&self) -> &str {
        "Search available skills by keyword. Matches against name, description, \
         tags, category, and author. Optionally filter by source or category."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to match against skill name, description, tags, category, or author."
                },
                "source": {
                    "type": "string",
                    "description": "Filter by upstream source (e.g. 'ClawHub', 'skills.sh', 'built-in', 'LobeHub')."
                },
                "category": {
                    "type": "string",
                    "description": "Filter by category."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let source_filter = args.get("source").and_then(|v| v.as_str());
        let category_filter = args.get("category").and_then(|v| v.as_str());

        tracing::debug!(
            query = %query,
            source = ?source_filter,
            category = ?category_filter,
            "[tool][skill_registry] search"
        );

        match ops::search_catalog(query, source_filter, category_filter).await {
            Ok(entries) => Ok(ToolResult::success(serde_json::to_string(&json!({
                "count": entries.len(),
                "entries": entries,
            }))?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to search skill catalog: {e}"
            ))),
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

pub struct SkillRegistryInstallTool {
    workspace_dir: PathBuf,
}

impl SkillRegistryInstallTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace_dir: config.workspace_dir.clone(),
        }
    }
}

#[async_trait]
impl Tool for SkillRegistryInstallTool {
    fn name(&self) -> &str {
        "skill_registry_install"
    }

    fn description(&self) -> &str {
        "Install a skill from the catalog by its entry_id. Downloads the \
         SKILL.md and installs it locally. Use `skill_registry_search` first \
         to find the entry to install."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entry_id": {
                    "type": "string",
                    "description": "The skill entry id (slug) to install."
                }
            },
            "required": ["entry_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    /// Installing a skill mutates the user's local skill set and fetches a
    /// remote `SKILL.md`, so it routes through the process-global
    /// `ApprovalGate` (#3993). On an interactive chat turn the user sees an
    /// inline approval card and approves before anything is written; on
    /// background/cron turns (no `APPROVAL_CHAT_CONTEXT`) the gate is bypassed,
    /// matching every other external-effect tool.
    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let entry_id = args
            .get("entry_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required argument `entry_id`"))?;

        tracing::debug!(entry_id = %entry_id, "[tool][skill_registry] install");

        let catalog = ops::browse_catalog(false)
            .await
            .map_err(|e| anyhow::anyhow!("failed to load catalog: {e}"))?;

        let entry = catalog.iter().find(|e| e.id == entry_id).ok_or_else(|| {
            anyhow::anyhow!(
                "skill '{entry_id}' not found in catalog. \
                     Run skill_registry_browse first to refresh."
            )
        })?;

        match ops::install_from_catalog(&self.workspace_dir, entry).await {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string(&json!({
                "url": outcome.url,
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
                "new_skills": outcome.new_skills,
            }))?)),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to install skill '{entry_id}': {e}"
            ))),
        }
    }
}

pub struct SkillRegistrySourcesTool;

#[async_trait]
impl Tool for SkillRegistrySourcesTool {
    fn name(&self) -> &str {
        "skill_registry_sources"
    }

    fn description(&self) -> &str {
        "List the distinct upstream sources available in the catalog \
         (e.g. 'built-in', 'ClawHub', 'skills.sh', 'LobeHub', 'browse.sh')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[tool][skill_registry] sources");
        match ops::list_sources().await {
            Ok(sources) => Ok(ToolResult::success(serde_json::to_string(&json!({
                "count": sources.len(),
                "sources": sources,
            }))?)),
            Err(e) => Ok(ToolResult::error(format!("Failed to list sources: {e}"))),
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

pub struct SkillRegistryUninstallTool;

#[async_trait]
impl Tool for SkillRegistryUninstallTool {
    fn name(&self) -> &str {
        "skill_registry_uninstall"
    }

    fn description(&self) -> &str {
        "Uninstall an installed user-scope skill by slug. Use after listing \
         installed workflows or when the user asks to remove a skill."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Installed skill slug to remove."
                }
            },
            "required": ["name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing required argument `name`"))?;
        tracing::debug!(name = %name, "[tool][skill_registry] uninstall");
        let params = crate::openhuman::workflows::ops_install::UninstallWorkflowParams {
            name: name.to_string(),
        };
        match crate::openhuman::workflows::ops_install::uninstall_workflow(params, None) {
            Ok(outcome) => Ok(ToolResult::success(serde_json::to_string(&json!({
                "name": outcome.name,
                "removed_path": outcome.removed_path,
                "scope": outcome.scope,
            }))?)),
            Err(error) => Ok(ToolResult::error(format!(
                "Failed to uninstall skill '{name}': {error}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_tool_is_external_effect_so_it_routes_through_approval_gate() {
        let tool = SkillRegistryInstallTool::new(Arc::new(Config::default()));
        assert_eq!(tool.name(), "skill_registry_install");
        // #3993: installs must raise an inline approval card before writing.
        assert!(
            tool.external_effect(),
            "skill_registry_install must declare external_effect so the harness gates it"
        );
        assert!(matches!(tool.permission_level(), PermissionLevel::Write));
    }

    #[test]
    fn read_only_skill_tools_are_not_gated() {
        // Browse/search/sources stay ungated — they only read the catalog.
        assert!(!SkillRegistryBrowseTool.external_effect());
        assert!(!SkillRegistrySearchTool.external_effect());
        assert!(!SkillRegistrySourcesTool.external_effect());
    }
}
