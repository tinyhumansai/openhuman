//! `todowrite` — lightweight todo-list state for multi-step runs.
//!
//! Coding-harness baseline tool (issue #1205). Each call replaces the
//! current todo list. Items have a `status` of `pending`, `in_progress`,
//! or `completed`. The list is process-global (one shared registry per
//! core) — sufficient as a baseline; per-session scoping can come later
//! once `task` carries a stable session id.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// Process-global todo state. Replaced wholesale on every call.
#[derive(Default)]
pub struct TodoStore {
    inner: Mutex<Vec<TodoItem>>,
}

impl TodoStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn replace(&self, items: Vec<TodoItem>) {
        *self.inner.lock() = items;
    }
    pub fn snapshot(&self) -> Vec<TodoItem> {
        self.inner.lock().clone()
    }
}

/// Process-global todo store. Returning the same `Arc` across calls
/// keeps todo state alive across registry rebuilds (the agent loop
/// can request a fresh tool registry without losing the running
/// todo list). Per-session scoping is a follow-up.
pub fn global_todo_store() -> Arc<TodoStore> {
    use once_cell::sync::OnceCell;
    static STORE: OnceCell<Arc<TodoStore>> = OnceCell::new();
    STORE.get_or_init(|| Arc::new(TodoStore::new())).clone()
}

pub struct TodoWriteTool {
    store: Arc<TodoStore>,
}

impl TodoWriteTool {
    pub fn new(store: Arc<TodoStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Replace the current todo list. Each item: `{content, status}` where \
         `status` is `pending`, `in_progress`, or `completed`. Returns a rendered \
         summary of the new list."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let todos = args
            .get("todos")
            .ok_or_else(|| anyhow::anyhow!("Missing 'todos' parameter"))?;
        let items: Vec<TodoItem> = serde_json::from_value(todos.clone())
            .map_err(|e| anyhow::anyhow!("Invalid todos array: {e}"))?;

        if items.iter().any(|i| i.content.trim().is_empty()) {
            return Ok(ToolResult::error("todo `content` must not be empty"));
        }

        let in_progress_count = items
            .iter()
            .filter(|i| i.status == TodoStatus::InProgress)
            .count();
        if in_progress_count > 1 {
            return Ok(ToolResult::error(format!(
                "Only one todo may be `in_progress` at a time (got {in_progress_count})"
            )));
        }

        self.store.replace(items.clone());

        let mut body = format!("Todo list updated ({} item(s)):", items.len());
        for item in &items {
            let mark = match item.status {
                TodoStatus::Completed => "[x]",
                TodoStatus::InProgress => "[~]",
                TodoStatus::Pending => "[ ]",
            };
            body.push('\n');
            body.push_str(&format!("{mark} {}", item.content));
        }
        Ok(ToolResult::success(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn todowrite_basic() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store.clone());
        let result = tool
            .execute(json!({
                "todos": [
                    { "content": "do A", "status": "pending" },
                    { "content": "do B", "status": "in_progress" },
                    { "content": "do C", "status": "completed" }
                ]
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let output = result.output();
        assert!(output.contains("[ ] do A"));
        assert!(output.contains("[~] do B"));
        assert!(output.contains("[x] do C"));
        let snap = store.snapshot();
        assert_eq!(snap.len(), 3);
    }

    #[tokio::test]
    async fn todowrite_replaces_state() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store.clone());
        tool.execute(json!({"todos": [{"content": "first", "status": "pending"}]}))
            .await
            .unwrap();
        tool.execute(json!({"todos": [{"content": "second", "status": "completed"}]}))
            .await
            .unwrap();
        let snap = store.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].content, "second");
    }

    #[tokio::test]
    async fn todowrite_rejects_multiple_in_progress() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let result = tool
            .execute(json!({
                "todos": [
                    { "content": "A", "status": "in_progress" },
                    { "content": "B", "status": "in_progress" }
                ]
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("in_progress"));
    }

    #[tokio::test]
    async fn todowrite_rejects_empty_content() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let result = tool
            .execute(json!({"todos": [{"content": "  ", "status": "pending"}]}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn todowrite_empty_list_is_allowed() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let result = tool.execute(json!({"todos": []})).await.unwrap();
        assert!(!result.is_error);
    }
}
