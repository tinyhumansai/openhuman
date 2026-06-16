//! Agent tools for managing the subconscious scratchpad.
//!
//! Three tools: `scratchpad_add`, `scratchpad_edit`, `scratchpad_remove`.
//! Registered via `all_scratchpad_tools()` and wired into the tool
//! registry so the subconscious agent can manage its own working memory.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::json;

async fn workspace_dir() -> anyhow::Result<std::path::PathBuf> {
    let config = crate::openhuman::config::load_config_with_timeout()
        .await
        .map_err(|e| anyhow::anyhow!("config load: {e}"))?;
    Ok(config.workspace_dir)
}

pub fn all_scratchpad_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ScratchpadAddTool),
        Box::new(ScratchpadEditTool),
        Box::new(ScratchpadRemoveTool),
    ]
}

// ── scratchpad_add ──────────────────────────────────────────────────────────

pub struct ScratchpadAddTool;

#[async_trait]
impl Tool for ScratchpadAddTool {
    fn name(&self) -> &str {
        "scratchpad_add"
    }

    fn description(&self) -> &str {
        "Add a thought to the subconscious scratchpad. Use this to persist \
         observations, hypotheses, or follow-up items across ticks. Max 100 entries; \
         oldest low-priority entries are evicted when the cap is reached."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["body"],
            "properties": {
                "body": {
                    "type": "string",
                    "description": "The thought or note to persist (keep under ~500 chars)."
                },
                "priority": {
                    "type": "integer",
                    "description": "Priority 0-10. Higher priority entries survive eviction longer. Default: 0.",
                    "minimum": 0,
                    "maximum": 10
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("scratchpad_add: `body` is required"))?;
        let priority = args
            .get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(10) as u32;

        let ws = workspace_dir().await?;
        let id = super::add(&ws, body, priority, super::DEFAULT_MAX_ENTRIES)?;

        Ok(ToolResult::success(format!(
            "Added scratchpad entry id={id} priority={priority}"
        )))
    }
}

// ── scratchpad_edit ─────────────────────────────────────────────────────────

pub struct ScratchpadEditTool;

#[async_trait]
impl Tool for ScratchpadEditTool {
    fn name(&self) -> &str {
        "scratchpad_edit"
    }

    fn description(&self) -> &str {
        "Edit an existing scratchpad entry. Update the body text and/or priority."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id", "body"],
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The entry ID to edit (shown in the scratchpad section as [id])."
                },
                "body": {
                    "type": "string",
                    "description": "Updated thought text."
                },
                "priority": {
                    "type": "integer",
                    "description": "Updated priority 0-10 (omit to keep current).",
                    "minimum": 0,
                    "maximum": 10
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("scratchpad_edit: `id` is required"))?;
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("scratchpad_edit: `body` is required"))?;
        let priority = args
            .get("priority")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(10) as u32);

        let ws = workspace_dir().await?;
        let found = super::edit(&ws, id, body, priority)?;

        if found {
            Ok(ToolResult::success(format!(
                "Updated scratchpad entry id={id}"
            )))
        } else {
            Ok(ToolResult::error(format!(
                "Scratchpad entry id={id} not found"
            )))
        }
    }
}

// ── scratchpad_remove ───────────────────────────────────────────────────────

pub struct ScratchpadRemoveTool;

#[async_trait]
impl Tool for ScratchpadRemoveTool {
    fn name(&self) -> &str {
        "scratchpad_remove"
    }

    fn description(&self) -> &str {
        "Remove a scratchpad entry by ID. Use when a thought is no longer relevant \
         or has been fully addressed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The entry ID to remove."
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("scratchpad_remove: `id` is required"))?;

        let ws = workspace_dir().await?;
        let found = super::remove(&ws, id)?;

        if found {
            Ok(ToolResult::success(format!(
                "Removed scratchpad entry id={id}"
            )))
        } else {
            Ok(ToolResult::error(format!(
                "Scratchpad entry id={id} not found"
            )))
        }
    }
}
