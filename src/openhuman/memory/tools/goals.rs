//! Agent-facing tools for the long-term goals list.
//!
//! These are the tools the background `goals_agent` (and, when allowed, the
//! main agent) uses to read and mutate the goals list over multiple turns.
//!
//! # They are wrappers around [`goals::ops`], not around a store (#5560)
//!
//! Each tool used to call `tinycortex::memory::goals::store` against a
//! `workspace_dir` captured at construction. Both halves of that changed:
//!
//! - The **store** is behind the loaded module now, reached through the goals
//!   family on the guarded driver.
//! - The **validation** — the secret/PII and single-line guards — is host
//!   policy that the family deliberately does not carry, and it lives in
//!   [`goals::doc`](crate::openhuman::memory::goals::doc).
//!
//! Routing through [`goals::ops`](crate::openhuman::memory::goals::ops) is what
//! keeps those two facts in one place. When these tools called the store
//! directly they duplicated the RPC surface's sequence, so the tool path and
//! the `memory_goals.*` path were two implementations of one operation; now
//! they are the same one, and a goal added by the agent is validated,
//! capped and reported exactly as a goal added over RPC.
//!
//! The constructors still take a `workspace_dir` and it is still what
//! `tools::ops` passes: it is the sandbox identity these tools were built with,
//! and dropping it would change four public signatures in the agent's tool
//! registry for no behavioural gain. It is unused for storage — the goals
//! document is workspace-wide on the driver's side too, so the tool and the
//! ambient binding name the same file.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::goals::ops;
use crate::openhuman::memory::guard::MemoryGuard;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// The guarded driver for this call, checked to serve the goals family.
///
/// Returned as the guard because the family accessor borrows from it. The
/// error text is what the agent reads back as a tool failure, so it names the
/// missing capability rather than an internal path.
async fn goals_guard() -> Result<std::sync::Arc<MemoryGuard>, String> {
    let guard = active_memory_guard().await?;
    if guard.as_goals().is_none() {
        return Err("memory driver does not support the goals family".to_string());
    }
    Ok(guard)
}

/// `goals_list` — read the current long-term goals list.
pub struct GoalsListTool {
    workspace_dir: PathBuf,
}

impl GoalsListTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalsListTool {
    fn name(&self) -> &str {
        "goals_list"
    }

    fn description(&self) -> &str {
        "List the user's current long-term goals. Returns each goal's id and \
         text. Always call this before adding/editing/deleting so you address \
         the right ids and avoid duplicates."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[memory_goals] tool=goals_list");
        let guard = match goals_guard().await {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let goals = guard.as_goals().expect("checked in goals_guard");
        match ops::list(goals).await {
            // `render()` is the contract type's own markdown, so the bytes the
            // agent reads are unchanged.
            Ok(outcome) => Ok(ToolResult::success(outcome.value.render())),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// `goals_add` — add a new long-term goal.
pub struct GoalsAddTool {
    workspace_dir: PathBuf,
}

impl GoalsAddTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalsAddTool {
    fn name(&self) -> &str {
        "goals_add"
    }

    fn description(&self) -> &str {
        "Add a new long-term goal (one concise sentence describing a durable \
         objective for working with the user). Returns the assigned goal id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "description": "The goal text — one concise sentence." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(text) = args.get("text").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'text' parameter"));
        };
        log::debug!("[memory_goals] tool=goals_add");
        let guard = match goals_guard().await {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let goals = guard.as_goals().expect("checked in goals_guard");
        match ops::add(goals, text).await {
            Ok(outcome) => Ok(ToolResult::success(format!(
                "Added goal '{}'.",
                outcome.value.id
            ))),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// `goals_edit` — replace the text of an existing goal.
pub struct GoalsEditTool {
    workspace_dir: PathBuf,
}

impl GoalsEditTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalsEditTool {
    fn name(&self) -> &str {
        "goals_edit"
    }

    fn description(&self) -> &str {
        "Edit an existing long-term goal by id, replacing its text. Use \
         goals_list first to find the id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id", "text"],
            "properties": {
                "id": { "type": "string", "description": "The goal id to edit (e.g. 'g1')." },
                "text": { "type": "string", "description": "The new goal text." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'id' parameter"));
        };
        let Some(text) = args.get("text").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'text' parameter"));
        };
        log::debug!("[memory_goals] tool=goals_edit id={id}");
        let guard = match goals_guard().await {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let goals = guard.as_goals().expect("checked in goals_guard");
        match ops::edit(goals, id, text).await {
            Ok(_) => Ok(ToolResult::success(format!("Edited goal '{id}'."))),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// `goals_delete` — remove a goal by id.
pub struct GoalsDeleteTool {
    workspace_dir: PathBuf,
}

impl GoalsDeleteTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalsDeleteTool {
    fn name(&self) -> &str {
        "goals_delete"
    }

    fn description(&self) -> &str {
        "Delete a long-term goal by id (e.g. when it is completed or no longer \
         relevant). Use goals_list first to find the id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": { "type": "string", "description": "The goal id to delete (e.g. 'g1')." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'id' parameter"));
        };
        log::debug!("[memory_goals] tool=goals_delete id={id}");
        let guard = match goals_guard().await {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        let goals = guard.as_goals().expect("checked in goals_guard");
        match ops::delete(goals, id).await {
            Ok(_) => Ok(ToolResult::success(format!("Deleted goal '{id}'."))),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

#[cfg(test)]
#[path = "goals_tests.rs"]
mod tests;
