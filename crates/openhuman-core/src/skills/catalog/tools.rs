//! LLM-callable tools for the skill registry domain.
//!
//! These tools let the orchestrator (and other agents) browse the aggregated
//! Hermes catalog, search for skills, and install from catalog entries.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::config::Config;
use crate::skills::ops_install::{ScanAcknowledgement, ScanBlockedOutcome, SkillInstallOutcome};
use crate::tools::status::{NOT_FOUND_MARKER, UNSUPPORTED_MARKER};
use tinytools::{PermissionLevel, Tool, ToolResult};

use super::ops;
use {
    super::types::CatalogPage, super::types::CatalogQuery, super::types::MAX_PAGE_SIZE,
    tinyskills::RegistryErrorKind,
};

/// Matches returned per browse/search call when the caller does not say:
/// enough candidates to compare and pick from in one read. A broad query over
/// the ~100k-entry catalog matches hundreds, and returning them all makes
/// every search a payload the harness has to summarize first.
const PAGE_DEFAULT_LIMIT: usize = 20;
/// Largest page a caller may ask for.
const PAGE_MAX_LIMIT: usize = MAX_PAGE_SIZE;

fn str_arg<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn usize_arg(args: &serde_json::Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
}

/// The catalog query a browse/search call asks for. `offset` is accepted for
/// callers that still page by it and mapped onto the page it falls in.
fn paged_query(args: &serde_json::Value, text: &str) -> CatalogQuery {
    let limit = usize_arg(args, "limit").map_or(PAGE_DEFAULT_LIMIT, |n| n.clamp(1, PAGE_MAX_LIMIT));
    let page = usize_arg(args, "page")
        .or_else(|| usize_arg(args, "offset").map(|offset| offset / limit + 1))
        .unwrap_or(1)
        .max(1);
    CatalogQuery {
        text: text.to_owned(),
        upstreams: str_arg(args, "source")
            .map(str::to_owned)
            .into_iter()
            .collect(),
        categories: str_arg(args, "category")
            .map(str::to_owned)
            .into_iter()
            .collect(),
        page: Some(page),
        page_size: Some(limit),
        force_refresh: args
            .get("force_refresh")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

fn page_result(page: CatalogPage) -> anyhow::Result<ToolResult> {
    let next_page = (page.page < page.total_pages).then_some(page.page + 1);
    tracing::debug!(
        total = page.total,
        page = page.page,
        returned = page.entries.len(),
        freshness = ?page.freshness,
        "[tool][skill_registry] page"
    );
    Ok(ToolResult::success(serde_json::to_string(&json!({
        "total": page.total,
        "page": page.page,
        "total_pages": page.total_pages,
        "next_page": next_page,
        "count": page.entries.len(),
        "freshness": page.freshness,
        "fetched_at": page.fetched_at,
        "refreshing": page.refreshing,
        "last_error": page.last_error,
        "entries": page.entries,
    }))?))
}

fn paging_schema() -> serde_json::Value {
    json!({
        "page": {
            "type": "integer",
            "minimum": 1,
            "default": 1,
            "description": "1-based page. Pass `next_page` from the previous result to read on."
        },
        "limit": {
            "type": "integer",
            "minimum": 1,
            "maximum": PAGE_MAX_LIMIT,
            "default": PAGE_DEFAULT_LIMIT,
            "description": "Matches per page."
        },
        "source": {
            "type": "string",
            "description": "Filter by upstream source (e.g. 'ClawHub', 'skills.sh', 'built-in', 'LobeHub')."
        },
        "category": {
            "type": "string",
            "description": "Filter by category."
        }
    })
}

fn with_paging(mut properties: serde_json::Value) -> serde_json::Value {
    if let (Some(target), serde_json::Value::Object(paging)) =
        (properties.as_object_mut(), paging_schema())
    {
        target.extend(paging);
    }
    properties
}

/// What the agent is told after the supply-chain scan refused an install.
pub(crate) const SCAN_BLOCKED_AGENT_INSTRUCTION: &str = "The security scan blocked this skill \
     and it was NOT installed. Do not retry the install. Tell the user what the scan found and \
     that, if they still want the skill, they can review the findings and install it themselves \
     from the Skills page.";

pub(crate) fn scan_blocked_tool_result(blocked: &ScanBlockedOutcome) -> anyhow::Result<ToolResult> {
    tracing::info!(
        target_id = %blocked.target,
        findings = blocked.findings.len(),
        "[tool][skill_registry] install refused by the scan"
    );
    Ok(ToolResult::error(serde_json::to_string(&json!({
        "status": "scan_blocked",
        "target": blocked.target,
        "findings": blocked.findings,
        "message": blocked.message,
        "instruction": SCAN_BLOCKED_AGENT_INSTRUCTION,
    }))?))
}

fn registry_tool_error(action: &str, error: &tinyskills::RegistryError) -> ToolResult {
    let marker = match error.kind() {
        RegistryErrorKind::NotFound | RegistryErrorKind::UnknownRegistry => {
            format!("{NOT_FOUND_MARKER} ")
        }
        RegistryErrorKind::NoDirectDownload | RegistryErrorKind::UpstreamAmbiguous => {
            format!("{UNSUPPORTED_MARKER} ")
        }
        _ => String::new(),
    };
    ToolResult::error(format!(
        "{marker}Failed to {action}: {}",
        ops::registry_error_message(error)
    ))
}

pub struct SkillRegistryBrowseTool;

#[async_trait]
impl Tool for SkillRegistryBrowseTool {
    fn name(&self) -> &str {
        "skill_registry_browse"
    }

    fn description(&self) -> &str {
        "Browse the aggregated skill catalog (HermesHub, ClawHub, skills.sh, \
         LobeHub, browse.sh) one page at a time (`limit`, default 20). Returns \
         the `total` count, `next_page`, and `freshness` (`cached` with a \
         `last_error` means a saved copy is shown). Use `force_refresh: true` \
         to refetch first."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": with_paging(json!({
                "force_refresh": {
                    "type": "boolean",
                    "description": "Refetch the catalog before answering.",
                    "default": false
                }
            }))
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = paged_query(&args, "");
        tracing::debug!(
            page = ?query.page,
            limit = ?query.page_size,
            force_refresh = query.force_refresh,
            "[tool][skill_registry] browse"
        );
        match ops::catalog_page(&query).await {
            Ok(page) => page_result(page),
            Err(error) => Ok(registry_tool_error("browse skill catalog", &error)),
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
         tags, category, and author. Optionally filter by source or category. \
         Returns one page of matches (`limit`, default 20) with the `total` \
         match count; pass `next_page` as `page` to read the next page."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": with_paging(json!({
                "query": {
                    "type": "string",
                    "description": "Search query to match against skill name, description, tags, category, or author."
                }
            })),
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let text = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let query = paged_query(&args, text);
        tracing::debug!(
            query = %query.text,
            source = ?query.upstreams,
            category = ?query.categories,
            page = ?query.page,
            limit = ?query.page_size,
            "[tool][skill_registry] search"
        );
        match ops::catalog_page(&query).await {
            Ok(page) => page_result(page),
            Err(error) => Ok(registry_tool_error("search skill catalog", &error)),
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
         SKILL.md, runs the security scan and installs it locally. Use \
         `skill_registry_search` first to find the entry to install. A \
         `scan_blocked` result means nothing was installed: tell the user, \
         who can choose to install it from the Skills page."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entry_id": {
                    "type": "string",
                    "description": "The `id` of the entry to install, exactly as returned by skill_registry_search (e.g. 'clawhub/apple-design')."
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
    /// `ApprovalGate`. On an interactive chat turn the user approves an inline
    /// card before anything is written; background turns bypass the gate,
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

        match ops::install_from_catalog(&self.workspace_dir, entry_id, ScanAcknowledgement::Absent)
            .await
        {
            Ok(SkillInstallOutcome::Installed(outcome)) => {
                Ok(ToolResult::success(serde_json::to_string(&json!({
                    "status": "installed",
                    "url": outcome.url,
                    "stdout": outcome.stdout,
                    "stderr": outcome.stderr,
                    "new_skills": outcome.new_skills,
                }))?))
            }
            Ok(SkillInstallOutcome::ScanBlocked(blocked)) => scan_blocked_tool_result(&blocked),
            Err(ops::CatalogInstallError::Registry(error)) => Ok(registry_tool_error(
                &format!("install skill '{entry_id}'"),
                &error,
            )),
            Err(error @ ops::CatalogInstallError::Install(_)) => Ok(ToolResult::error(format!(
                "Failed to install skill '{entry_id}': {error}"
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
         (e.g. 'built-in', 'ClawHub', 'skills.sh', 'LobeHub', 'browse.sh'), \
         most entries first, with their entry counts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[tool][skill_registry] sources");
        match ops::catalog_facets().await {
            Ok(facets) => {
                let sources: Vec<&str> =
                    facets.upstreams.iter().map(|f| f.value.as_str()).collect();
                Ok(ToolResult::success(serde_json::to_string(&json!({
                    "count": sources.len(),
                    "sources": sources,
                    "counts": facets.upstreams,
                    "freshness": facets.freshness,
                }))?))
            }
            Err(error) => Ok(registry_tool_error("list sources", &error)),
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
        let params = crate::skills::ops_install::UninstallWorkflowParams {
            name: name.to_string(),
        };
        match crate::skills::ops_install::uninstall_workflow(params, None) {
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
#[path = "tools_tests.rs"]
mod tests;
