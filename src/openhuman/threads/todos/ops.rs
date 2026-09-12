//! Compatibility facade over [`tinyagents_graph::todos`].
//!
//! OpenHuman keeps its historical board-location and optional-thread snapshot
//! shapes for RPC and tool callers. All task-board data, normalization, CRUD,
//! and compare-and-set behavior is owned by TinyAgents.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tinyagents_graph::todos::store as todos;
use tinyagents_harness::store::Store;

use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::task_board::normalize_cards_for_wire;
pub use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::agent::tinyagents::todos::{
    scratch_todos_store, todos_store, SCRATCH_THREAD_ID,
};

pub const USER_TASKS_THREAD_ID: &str = "user-tasks";
pub const ORCHESTRATOR_TASKS_THREAD_ID: &str = "orchestrator-tasks";

pub use tinyagents_graph::todos::{parse_status, render_markdown, CardPatch};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodosSnapshot {
    pub thread_id: Option<String>,
    pub cards: Vec<TaskBoardCard>,
    pub markdown: String,
}

#[derive(Debug, Clone)]
pub enum BoardLocation {
    Thread {
        workspace_dir: PathBuf,
        thread_id: String,
    },
    Scratch,
}

impl BoardLocation {
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::Thread { thread_id, .. } => Some(thread_id),
            Self::Scratch => None,
        }
    }
}

pub(super) fn target(location: &BoardLocation) -> (Arc<dyn Store>, &str) {
    match location {
        BoardLocation::Thread {
            workspace_dir,
            thread_id,
        } => (todos_store(workspace_dir), thread_id),
        BoardLocation::Scratch => (scratch_todos_store(), SCRATCH_THREAD_ID),
    }
}

fn snapshot(
    location: &BoardLocation,
    value: tinyagents_graph::todos::TodosSnapshot,
) -> TodosSnapshot {
    TodosSnapshot {
        thread_id: location.thread_id().map(str::to_owned),
        cards: value.cards,
        markdown: value.markdown,
    }
}

fn progress_updated_at() -> String {
    Utc::now().to_rfc3339()
}

fn emit_progress(location: &BoardLocation, cards: &[TaskBoardCard]) {
    let BoardLocation::Thread { thread_id, .. } = location else {
        return;
    };
    let Some(parent) = crate::openhuman::agent::harness::fork_context::current_parent() else {
        return;
    };
    let Some(tx) = parent.on_progress else {
        return;
    };
    let board = tinyagents_graph::todos::TaskBoard {
        thread_id: thread_id.clone(),
        cards: cards.to_vec(),
        updated_at: progress_updated_at(),
    };
    if let Err(error) = tx.try_send(AgentProgress::TaskBoardUpdated { board }) {
        tracing::debug!(thread_id, %error, "task board progress dropped");
    }
}

fn finish(
    location: &BoardLocation,
    result: tinyagents_harness::error::Result<tinyagents_graph::todos::TodosSnapshot>,
) -> Result<TodosSnapshot, String> {
    let mut value = result.map_err(|error| error.to_string())?;
    normalize_cards_for_wire(&mut value.cards);
    emit_progress(location, &value.cards);
    Ok(snapshot(location, value))
}

pub async fn add(
    location: &BoardLocation,
    content: &str,
    patch: CardPatch,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(
        location,
        todos::add(&store, thread_id, content, patch).await,
    )
}

pub async fn edit(
    location: &BoardLocation,
    id: &str,
    patch: CardPatch,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(location, todos::edit(&store, thread_id, id, patch).await)
}

pub async fn update_status(
    location: &BoardLocation,
    id: &str,
    status: TaskCardStatus,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(
        location,
        todos::update_status(&store, thread_id, id, status).await,
    )
}

pub async fn set_session_thread(
    location: &BoardLocation,
    id: &str,
    session_thread_id: Option<String>,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(
        location,
        todos::set_session_thread(&store, thread_id, id, session_thread_id).await,
    )
}

pub async fn decide_plan(
    location: &BoardLocation,
    id: &str,
    approve: bool,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(
        location,
        todos::decide_plan(&store, thread_id, id, approve).await,
    )
}

pub async fn revise_plan(
    location: &BoardLocation,
    feedback: &str,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    tracing::info!(
        thread_id,
        feedback_len = feedback.len(),
        "[todos][ops] revise_plan requested re-plan"
    );
    finish(location, todos::revise_plan(&store, thread_id).await)
}

pub async fn remove(location: &BoardLocation, id: &str) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(location, todos::remove(&store, thread_id, id).await)
}

pub async fn replace(
    location: &BoardLocation,
    cards: Vec<TaskBoardCard>,
) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(location, todos::replace(&store, thread_id, cards).await)
}

pub async fn clear(location: &BoardLocation) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    finish(location, todos::clear(&store, thread_id).await)
}

pub async fn list(location: &BoardLocation) -> Result<TodosSnapshot, String> {
    let (store, thread_id) = target(location);
    todos::list(&store, thread_id)
        .await
        .map(|value| snapshot(location, value))
        .map_err(|error| error.to_string())
}

pub async fn claim_card(
    location: &BoardLocation,
    card_id: &str,
    expected: &[TaskCardStatus],
    target_status: TaskCardStatus,
) -> Result<TaskBoardCard, String> {
    let (store, thread_id) = target(location);
    let card = todos::claim_card(&store, thread_id, card_id, expected, target_status)
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(value) = todos::list(&store, thread_id).await {
        emit_progress(location, &value.cards);
    }
    Ok(card)
}

#[cfg(test)]
pub(crate) fn scratch_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
