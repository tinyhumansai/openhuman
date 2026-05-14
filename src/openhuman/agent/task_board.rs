//! Persistent per-thread task board used by the agent kanban UI.
//!
//! Boards live under `<workspace>/agent_task_boards/<hex(thread_id)>.json`.
//! The agent updates them through the `todowrite` tool; the UI can fetch or
//! replace them through the thread RPC surface.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const TASK_BOARD_DIR: &str = "agent_task_boards";
const TASK_BOARD_EXTENSION: &str = "json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCardStatus {
    Todo,
    InProgress,
    Blocked,
    Done,
}

impl TaskCardStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoardCard {
    pub id: String,
    pub title: String,
    pub status: TaskCardStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoard {
    pub thread_id: String,
    pub cards: Vec<TaskBoardCard>,
    pub updated_at: String,
}

impl TaskBoard {
    pub fn empty(thread_id: impl Into<String>) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            thread_id: thread_id.into(),
            cards: Vec::new(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskBoardStore {
    workspace_dir: PathBuf,
}

impl TaskBoardStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    pub fn get(&self, thread_id: &str) -> Result<Option<TaskBoard>, String> {
        let path = self.board_path(thread_id);
        if !path.exists() {
            return Ok(None);
        }
        let mut buf = String::new();
        fs::File::open(&path)
            .map_err(|e| format!("open task board {}: {e}", path.display()))?
            .read_to_string(&mut buf)
            .map_err(|e| format!("read task board {}: {e}", path.display()))?;
        serde_json::from_str(&buf).map(Some).map_err(|e| {
            format!(
                "parse task board {} for thread '{}': {e}",
                path.display(),
                thread_id
            )
        })
    }

    pub fn put(&self, mut board: TaskBoard) -> Result<TaskBoard, String> {
        normalise_board(&mut board);
        let dir = self.ensure_dir()?;
        let path = self.board_path(&board.thread_id);
        let mut tmp = tempfile::NamedTempFile::new_in(&dir)
            .map_err(|e| format!("create task board tempfile in {}: {e}", dir.display()))?;
        let bytes =
            serde_json::to_vec_pretty(&board).map_err(|e| format!("serialize task board: {e}"))?;
        tmp.write_all(&bytes)
            .map_err(|e| format!("write task board tempfile: {e}"))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| format!("fsync task board tempfile: {e}"))?;
        tmp.persist(&path)
            .map_err(|e| format!("persist task board {}: {e}", path.display()))?;
        Ok(board)
    }

    pub fn delete(&self, thread_id: &str) -> Result<bool, String> {
        let path = self.board_path(thread_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path).map_err(|e| format!("delete task board {}: {e}", path.display()))?;
        Ok(true)
    }

    fn ensure_dir(&self) -> Result<PathBuf, String> {
        let dir = self.workspace_dir.join(TASK_BOARD_DIR);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create task board dir {}: {e}", dir.display()))?;
        Ok(dir)
    }

    fn board_path(&self, thread_id: &str) -> PathBuf {
        self.workspace_dir.join(TASK_BOARD_DIR).join(format!(
            "{}.{}",
            hex::encode(thread_id.as_bytes()),
            TASK_BOARD_EXTENSION
        ))
    }
}

pub fn board_for_thread(workspace_dir: &Path, thread_id: &str) -> Result<TaskBoard, String> {
    let store = TaskBoardStore::new(workspace_dir.to_path_buf());
    Ok(store
        .get(thread_id)?
        .unwrap_or_else(|| TaskBoard::empty(thread_id)))
}

pub fn normalise_board(board: &mut TaskBoard) {
    board.thread_id = board.thread_id.trim().to_string();
    let now = Utc::now().to_rfc3339();
    board.updated_at = now.clone();

    for (idx, card) in board.cards.iter_mut().enumerate() {
        card.title = card.title.trim().to_string();
        if card.id.trim().is_empty() {
            card.id = format!("task-{}", uuid::Uuid::new_v4());
        } else {
            card.id = card.id.trim().to_string();
        }
        card.notes = card
            .notes
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        card.blocker = card
            .blocker
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if card.status == TaskCardStatus::Blocked && card.blocker.is_none() {
            card.blocker = card.notes.clone();
        }
        card.order = idx as u32;
        card.updated_at = now.clone();
    }

    board.cards.retain(|card| !card.title.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn board_store_roundtrips_and_normalises_cards() {
        let dir = tempdir().expect("tempdir");
        let store = TaskBoardStore::new(dir.path().to_path_buf());
        let board = TaskBoard {
            thread_id: "thread-1".into(),
            cards: vec![
                TaskBoardCard {
                    id: String::new(),
                    title: "  Draft plan  ".into(),
                    status: TaskCardStatus::Todo,
                    notes: Some("  note  ".into()),
                    blocker: None,
                    order: 99,
                    updated_at: String::new(),
                },
                TaskBoardCard {
                    id: "blocked".into(),
                    title: "Need approval".into(),
                    status: TaskCardStatus::Blocked,
                    notes: Some("waiting on user".into()),
                    blocker: None,
                    order: 99,
                    updated_at: String::new(),
                },
            ],
            updated_at: String::new(),
        };

        let saved = store.put(board).expect("put");
        assert_eq!(saved.cards[0].title, "Draft plan");
        assert_eq!(saved.cards[0].order, 0);
        assert!(saved.cards[0].id.starts_with("task-"));
        assert_eq!(saved.cards[1].blocker.as_deref(), Some("waiting on user"));

        let loaded = store.get("thread-1").expect("get").expect("present");
        assert_eq!(loaded.cards.len(), 2);
        assert_eq!(loaded.cards[1].status, TaskCardStatus::Blocked);
    }

    #[test]
    fn missing_board_returns_none() {
        let dir = tempdir().expect("tempdir");
        let store = TaskBoardStore::new(dir.path().to_path_buf());
        assert!(store.get("missing").expect("get").is_none());
    }
}
