//! OpenHuman integration for the TinyAgents task-board implementation.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use tinyagents_graph::todos::{store as todos, TaskBoard};
use tinyagents_harness::store::{InMemoryStore, Store};

use crate::openhuman::agent::session_import::ops::open_session_stores;

/// Open the durable TinyAgents store used by per-thread task boards.
pub fn todos_store(workspace_dir: &Path) -> Arc<dyn Store> {
    Arc::new(open_session_stores(workspace_dir).kv)
}

/// Shared ephemeral TinyAgents store used when a tool has no thread context.
pub fn scratch_todos_store() -> Arc<dyn Store> {
    static STORE: OnceLock<Arc<dyn Store>> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(InMemoryStore::new())).clone()
}

/// Synthetic thread key for the process-global scratch board.
pub const SCRATCH_THREAD_ID: &str = "_scratch_";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskBoardMigrationReport {
    pub total: usize,
    pub copied: usize,
    pub skipped: usize,
}

async fn read_legacy_boards(workspace_dir: &Path) -> Result<Vec<TaskBoard>, String> {
    let dir = workspace_dir.join("agent_task_boards");
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "read legacy task boards dir {}: {error}",
                dir.display()
            ))
        }
    };
    let mut boards = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("iterate legacy task boards dir: {error}"))?
    {
        let path = entry.path();
        let is_board = path.extension().and_then(|value| value.to_str()) == Some("json")
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".runs.json"));
        if !is_board {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(body) => match serde_json::from_str(&body) {
                Ok(board) => boards.push(board),
                Err(error) => {
                    tracing::debug!(path = %path.display(), %error, "skip invalid legacy task board")
                }
            },
            Err(error) => {
                tracing::debug!(path = %path.display(), %error, "skip unreadable legacy task board")
            }
        }
    }
    Ok(boards)
}

/// Copy boards from the retired file store without replacing TinyAgents data.
pub async fn migrate_legacy_task_boards(
    workspace_dir: &Path,
) -> Result<TaskBoardMigrationReport, String> {
    let legacy = read_legacy_boards(workspace_dir).await?;
    let store = todos_store(workspace_dir);
    let mut report = TaskBoardMigrationReport {
        total: legacy.len(),
        ..Default::default()
    };
    for board in legacy {
        if todos::import_if_absent(&store, board)
            .await
            .map_err(|error| error.to_string())?
        {
            report.copied += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
#[path = "todos_tests.rs"]
mod tests;
