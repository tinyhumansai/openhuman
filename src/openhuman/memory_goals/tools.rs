//! Agent-facing tools for the long-term goals list.
//!
//! These are the tools the background `goals_agent` (and, when allowed, the
//! main agent) uses to read and mutate the goals list over multiple turns.
//! They are thin wrappers around [`super::store`] — all cap enforcement and
//! persistence live there. Each tool is sandboxed to a single `workspace_dir`
//! captured at construction time.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use super::store;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

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
        let doc = match store::load(&self.workspace_dir).await {
            Ok(doc) => doc,
            Err(e) => return Ok(ToolResult::error(e)),
        };
        Ok(ToolResult::success(doc.render()))
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
        match store::add(&self.workspace_dir, text).await {
            Ok((id, _)) => Ok(ToolResult::success(format!("Added goal '{id}'."))),
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
        match store::edit(&self.workspace_dir, id, text).await {
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
        match store::delete(&self.workspace_dir, id).await {
            Ok(_) => Ok(ToolResult::success(format!("Deleted goal '{id}'."))),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_then_list_reflects_change() {
        let tmp = tempfile::tempdir().unwrap();
        let add = GoalsAddTool::new(tmp.path().to_path_buf());
        let res = add
            .execute(json!({ "text": "help ship the app" }))
            .await
            .unwrap();
        assert!(!res.is_error);

        let list = GoalsListTool::new(tmp.path().to_path_buf());
        let res = list.execute(json!({})).await.unwrap();
        assert!(res.text().contains("help ship the app"));
    }

    #[tokio::test]
    async fn edit_and_delete_unknown_id_error() {
        let tmp = tempfile::tempdir().unwrap();
        let edit = GoalsEditTool::new(tmp.path().to_path_buf());
        let res = edit
            .execute(json!({ "id": "g9", "text": "x" }))
            .await
            .unwrap();
        assert!(res.is_error);

        let del = GoalsDeleteTool::new(tmp.path().to_path_buf());
        let res = del.execute(json!({ "id": "g9" })).await.unwrap();
        assert!(res.is_error);
    }
}
