//! Persistent per-thread task board used by the agent kanban UI.
//!
//! **Crate-backed.** Boards now live in the vendored `tinyagents_graph::todos`
//! crate KV store (`<workspace>/tinyagents_store/kv/graph.todos/<hex(thread_id)>`),
//! not the retired `<workspace>/agent_task_boards/<hex(thread_id)>.json`
//! file-JSON tree. [`TaskBoardStore`] is a thin adapter that preserves the
//! historical `get`/`put`/`delete` surface every consumer uses and forwards each
//! operation directly to `tinyagents_graph::todos`.
//!
//! The agent updates boards through the `todo` tool; the UI can fetch or replace
//! them through the `threads.task_board_*` and granular `openhuman.todos_*` RPC
//! surfaces. The core process remains the single writer.

use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use tinyagents_graph::todos::store as crate_todos;
pub use tinyagents_graph::todos::{
    normalise_board, TaskApprovalMode, TaskBoard, TaskBoardCard, TaskCardStatus,
};

use crate::openhuman::agent::tinyagents::todos::todos_store;

#[derive(Debug, Clone)]
pub struct TaskBoardStore {
    workspace_dir: PathBuf,
}

impl TaskBoardStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    /// The thread's board, or `None` when the thread has never had one. Reads
    /// the crate `graph.todos` value **raw** (no normalise) so the absent-vs-
    /// empty distinction survives, exactly as the legacy file read did.
    pub async fn get(&self, thread_id: &str) -> Result<Option<TaskBoard>, String> {
        let thread_id = validate_thread_id(thread_id)?;
        tracing::debug!(thread_id = %thread_id, "[agent:task_board] get entry");
        let store = todos_store(&self.workspace_dir);
        match crate_todos::get(&store, &thread_id)
            .await
            .map_err(|error| format!("decode crate task board for thread {thread_id}: {error}"))?
        {
            Some(board) => {
                let board = normalize_board_for_wire(board);
                tracing::debug!(
                    thread_id = %thread_id,
                    card_count = board.cards.len(),
                    "[agent:task_board] get ok"
                );
                Ok(Some(board))
            }
            None => {
                tracing::debug!(thread_id = %thread_id, "[agent:task_board] get not_found");
                Ok(None)
            }
        }
    }

    /// Persist `board`. Hands the cards directly to the crate `replace` op,
    /// which owns normalisation and **enforces the single-`InProgress`
    /// invariant** (an invalid board now errors here rather than being silently
    /// saved). Returns the saved board with crate-normalised cards.
    pub async fn put(&self, board: TaskBoard) -> Result<TaskBoard, String> {
        tracing::debug!(
            thread_id = %board.thread_id,
            card_count = board.cards.len(),
            "[agent:task_board] put entry"
        );
        let thread_id = validate_thread_id(&board.thread_id)?;
        let store = todos_store(&self.workspace_dir);
        let snap = crate_todos::replace(&store, &thread_id, board.cards)
            .await
            .map_err(|e| e.to_string())?;
        let mut cards = snap.cards;
        normalize_cards_for_wire(&mut cards);
        tracing::debug!(
            thread_id = %thread_id,
            card_count = cards.len(),
            "[agent:task_board] put ok"
        );
        Ok(TaskBoard {
            thread_id,
            cards,
            updated_at: Utc::now().to_rfc3339(),
        })
    }

    /// Delete the thread's board. Returns whether a board was present. Removes
    /// the crate key outright (not the crate `clear`, which writes an empty
    /// board) so a subsequent [`get`](Self::get) reports `None`.
    pub async fn delete(&self, thread_id: &str) -> Result<bool, String> {
        let thread_id = validate_thread_id(thread_id)?;
        tracing::debug!(thread_id = %thread_id, "[agent:task_board] delete entry");
        let store = todos_store(&self.workspace_dir);
        let existed = crate_todos::delete(&store, &thread_id)
            .await
            .map_err(|e| e.to_string())?;
        tracing::debug!(thread_id = %thread_id, existed, "[agent:task_board] delete ok");
        Ok(existed)
    }
}

pub async fn board_for_thread(workspace_dir: &Path, thread_id: &str) -> Result<TaskBoard, String> {
    let thread_id = validate_thread_id(thread_id)?;
    let store = TaskBoardStore::new(workspace_dir.to_path_buf());
    Ok(store
        .get(&thread_id)
        .await?
        .unwrap_or_else(|| normalize_board_for_wire(TaskBoard::empty(thread_id))))
}

pub(crate) fn normalize_timestamp_for_wire(value: &str) -> String {
    if chrono::DateTime::parse_from_rfc3339(value).is_ok() {
        return value.to_owned();
    }
    if let Ok(updated_at_ms) = value.parse::<i64>() {
        if let Some(updated_at) = Utc.timestamp_millis_opt(updated_at_ms).single() {
            return updated_at.to_rfc3339();
        }
    }
    tracing::warn!(updated_at = %value, "invalid task-board timestamp; using current time");
    Utc::now().to_rfc3339()
}

pub(crate) fn normalize_cards_for_wire(cards: &mut [TaskBoardCard]) {
    for card in cards {
        card.updated_at = normalize_timestamp_for_wire(&card.updated_at);
    }
}

fn normalize_board_for_wire(mut board: TaskBoard) -> TaskBoard {
    board.updated_at = normalize_timestamp_for_wire(&board.updated_at);
    normalize_cards_for_wire(&mut board.cards);
    board
}

fn validate_thread_id(thread_id: &str) -> Result<String, String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err("invalid task board thread_id: empty or whitespace".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "task_board_tests.rs"]
mod tests;
