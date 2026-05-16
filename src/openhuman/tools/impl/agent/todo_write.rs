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
        let items: Vec<TodoItem> = items.into_iter().map(normalize_todo_item).collect();

        if items.iter().any(|i| i.content.is_empty()) {
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
        match persisted_board {
            Ok(()) => {}
            Err(TaskBoardPersistError::MissingContext(reason)) => {
                tracing::debug!(reason, "[todowrite] task board persistence skipped");
            }
            Err(TaskBoardPersistError::Persist(err)) => {
                tracing::debug!(
                    error = %err,
                    "[todowrite] task board persistence failed"
                );
                return Ok(ToolResult::error(format!(
                    "Failed to persist task board: {err}"
                )));
            }
        }
        Ok(ToolResult::success(body))
    }
}

fn normalize_todo_item(mut item: TodoItem) -> TodoItem {
    item.content = item.content.trim().to_string();
    item.id = normalize_optional(item.id);
    item.notes = normalize_optional(item.notes);
    item.blocker = normalize_optional(item.blocker);
    item
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug)]
enum TaskBoardPersistError {
    MissingContext(&'static str),
    Persist(String),
}

impl std::fmt::Display for TaskBoardPersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContext(reason) => write!(f, "{reason}"),
            Self::Persist(err) => write!(f, "{err}"),
        }
    }
}

async fn persist_thread_board(items: &[TodoItem]) -> Result<(), TaskBoardPersistError> {
    let parent =
        current_parent().ok_or(TaskBoardPersistError::MissingContext("no parent context"))?;
    let thread_id = thread_context::current_thread_id()
        .ok_or(TaskBoardPersistError::MissingContext("no thread id"))?;
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
    let saved = TaskBoardStore::new(parent.workspace_dir.clone())
        .put(board)
        .map_err(TaskBoardPersistError::Persist)?;
    let workspace_name = parent
        .workspace_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unknown>".to_string());
    tracing::debug!(
        thread_id = %saved.thread_id,
        workspace = %workspace_name,
        card_count = saved.cards.len(),
        "[todowrite][task_board] persisted"
    );
    if let Some(tx) = parent.on_progress {
        if let Err(err) = tx.try_send(AgentProgress::TaskBoardUpdated {
            board: saved.clone(),
        }) {
            tracing::debug!(
                thread_id = %saved.thread_id,
                error = %err,
                "[todowrite] task board progress dropped"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::fork_context::{
        with_parent_context, ParentExecutionContext,
    };
    use crate::openhuman::context::prompt::ToolCallFormat;
    use crate::openhuman::memory::{
        Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
    };
    use crate::openhuman::providers::thread_context::with_thread_id;
    use crate::openhuman::providers::{ChatRequest, ChatResponse, Provider};

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

    #[tokio::test]
    async fn todowrite_normalizes_items_before_store_and_output() {
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store.clone());
        let result = tool
            .execute(json!({
                "todos": [
                    {
                        "id": "   ",
                        "content": " Draft plan ",
                        "status": "blocked",
                        "notes": "  ",
                        "blocker": " waiting "
                    }
                ]
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().contains("[!] Draft plan"));
        assert!(result.output().contains("blocked: waiting"));
        assert!(!result.output().contains("[!]  Draft plan"));

        let snap = store.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].content, "Draft plan");
        assert_eq!(snap[0].id, None);
        assert_eq!(snap[0].notes, None);
        assert_eq!(snap[0].blocker.as_deref(), Some("waiting"));
    }

    struct NoopProvider;

    #[async_trait]
    impl Provider for NoopProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                text: Some("ok".into()),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    struct NoopMemory;

    #[async_trait]
    impl Memory for NoopMemory {
        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }

        fn name(&self) -> &str {
            "noop"
        }
    }

    fn parent_context(
        workspace_dir: std::path::PathBuf,
        on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    ) -> ParentExecutionContext {
        ParentExecutionContext {
            provider: Arc::new(NoopProvider),
            all_tools: Arc::new(Vec::new()),
            all_tool_specs: Arc::new(Vec::new()),
            model_name: "test-model".into(),
            temperature: 0.2,
            workspace_dir,
            memory: Arc::new(NoopMemory),
            agent_config: crate::openhuman::config::AgentConfig::default(),
            skills: Arc::new(Vec::new()),
            memory_context: Arc::new(None),
            session_id: "session-test".into(),
            channel: "test".into(),
            connected_integrations: Vec::new(),
            tool_call_format: ToolCallFormat::PFormat,
            session_key: "0_test".into(),
            session_parent_prefix: None,
            on_progress,
        }
    }

    #[tokio::test]
    async fn todowrite_persists_active_thread_board_and_emits_progress() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let parent = parent_context(temp.path().to_path_buf(), Some(tx));

        let result = with_thread_id("thread-todo", async {
            with_parent_context(parent, async {
                tool.execute(json!({
                    "todos": [
                        {
                            "id": " task-1 ",
                            "content": " Draft plan ",
                            "status": "pending",
                            "notes": " note "
                        },
                        {
                            "content": "Wait for approval",
                            "status": "blocked",
                            "notes": "needs sign-off"
                        }
                    ]
                }))
                .await
            })
            .await
        })
        .await
        .expect("todowrite execute");
        assert!(!result.is_error, "{}", result.output());

        let saved = TaskBoardStore::new(temp.path().to_path_buf())
            .get("thread-todo")
            .expect("load persisted board")
            .expect("board exists");
        assert_eq!(saved.cards.len(), 2);
        assert_eq!(saved.cards[0].id, "task-1");
        assert_eq!(saved.cards[0].status, TaskCardStatus::Todo);
        assert_eq!(saved.cards[1].status, TaskCardStatus::Blocked);
        assert_eq!(saved.cards[1].blocker.as_deref(), Some("needs sign-off"));

        let progress = rx.try_recv().expect("task board progress event");
        match progress {
            AgentProgress::TaskBoardUpdated { board } => {
                assert_eq!(board.thread_id, "thread-todo");
                assert_eq!(board.cards.len(), 2);
            }
            other => panic!("unexpected progress event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn todowrite_does_not_block_when_progress_channel_full() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(AgentProgress::TaskBoardUpdated {
            board: TaskBoard::empty("pre-filled"),
        })
        .expect("pre-fill progress channel");
        let parent = parent_context(temp.path().to_path_buf(), Some(tx));

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            with_thread_id("thread-todo", async {
                with_parent_context(parent, async {
                    tool.execute(json!({
                        "todos": [
                            { "content": "Draft plan", "status": "pending" }
                        ]
                    }))
                    .await
                })
                .await
            })
            .await
        })
        .await
        .expect("todowrite should not block on a full progress channel")
        .expect("todowrite execute");

        assert!(!result.is_error, "{}", result.output());
    }

    #[tokio::test]
    async fn todowrite_reports_task_board_persistence_failures() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("agent_task_boards"), b"not a directory")
            .expect("blocking task board path");
        let store = Arc::new(TodoStore::new());
        let tool = TodoWriteTool::new(store);
        let parent = parent_context(temp.path().to_path_buf(), None);

        let result = with_thread_id("thread-todo", async {
            with_parent_context(parent, async {
                tool.execute(json!({
                    "todos": [
                        { "content": "Draft plan", "status": "pending" }
                    ]
                }))
                .await
            })
            .await
        })
        .await
        .expect("todowrite execute");

        assert!(result.is_error);
        assert!(result.output().contains("Failed to persist task board"));
        assert!(result.output().contains("create task board dir"));
    }
}
