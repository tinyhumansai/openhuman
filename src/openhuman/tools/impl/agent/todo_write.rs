//! `todowrite` — lightweight task-board state for multi-step runs.
//!
//! Each call replaces the current list and, when running inside a web
//! thread, persists the same cards as that thread's kanban board.

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::task_board::{
    TaskBoard, TaskBoardCard, TaskBoardStore, TaskCardStatus,
};
use crate::openhuman::providers::thread_context;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[serde(alias = "todo")]
    Pending,
    InProgress,
    Blocked,
    #[serde(alias = "done")]
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub content: String,
    pub status: TodoStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
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
        "Replace the current task board. Each item: `{content, status, notes?, blocker?}` \
         where `status` is `todo`/`pending`, `in_progress`, `blocked`, or \
         `done`/`completed`. Use `blocked` with a short blocker when work cannot proceed. \
         Returns a rendered summary and persists the board for the active thread."
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
                                "enum": ["todo", "pending", "in_progress", "blocked", "done", "completed"]
                            },
                            "id": { "type": "string" },
                            "notes": { "type": "string" },
                            "blocker": { "type": "string" }
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

        let persisted_board = persist_thread_board(&items).await;

        let mut body = format!("Todo list updated ({} item(s)):", items.len());
        for item in &items {
            let mark = match item.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[~]",
                TodoStatus::Blocked => "[!]",
                TodoStatus::Completed => "[x]",
            };
            body.push('\n');
            body.push_str(&format!("{mark} {}", item.content));
            if item.status == TodoStatus::Blocked {
                if let Some(reason) = item.blocker.as_deref().or(item.notes.as_deref()) {
                    body.push_str(&format!(" — blocked: {reason}"));
                }
            }
        }
        if let Err(err) = persisted_board {
            tracing::debug!(
                error = %err,
                "[todowrite] task board persistence skipped/failed"
            );
        }
        Ok(ToolResult::success(body))
    }
}

async fn persist_thread_board(items: &[TodoItem]) -> Result<(), String> {
    let parent = current_parent().ok_or_else(|| "no parent context".to_string())?;
    let thread_id =
        thread_context::current_thread_id().ok_or_else(|| "no thread id".to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let cards = items
        .iter()
        .enumerate()
        .map(|(idx, item)| TaskBoardCard {
            id: item
                .id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("task-{}", uuid::Uuid::new_v4())),
            title: item.content.trim().to_string(),
            status: match item.status {
                TodoStatus::Pending => TaskCardStatus::Todo,
                TodoStatus::InProgress => TaskCardStatus::InProgress,
                TodoStatus::Blocked => TaskCardStatus::Blocked,
                TodoStatus::Completed => TaskCardStatus::Done,
            },
            notes: item
                .notes
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            blocker: item
                .blocker
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            order: idx as u32,
            updated_at: now.clone(),
        })
        .collect();

    let board = TaskBoard {
        thread_id,
        cards,
        updated_at: now,
    };
    let saved = TaskBoardStore::new(parent.workspace_dir.clone()).put(board)?;
    if let Some(tx) = parent.on_progress {
        let _ = tx
            .send(AgentProgress::TaskBoardUpdated { board: saved })
            .await;
    }
    Ok(())
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

    #[tokio::test]
    async fn todowrite_renders_blockers() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let result = tool
            .execute(json!({
                "todos": [
                    { "content": "wait for credentials", "status": "blocked", "blocker": "missing token" }
                ]
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().contains("[!] wait for credentials"));
        assert!(result.output().contains("missing token"));
    }
}
