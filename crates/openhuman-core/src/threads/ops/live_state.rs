//! Read-back RPCs for the two thread-scoped, agent-driven surfaces a client
//! needs to hydrate on load or reconnect: the thread's goal (`agent::goals`)
//! and its session todo list (`agent::todos`). Live updates for both stream
//! separately over the web channel (`thread_goal_updated`/`thread_goal_cleared`,
//! `thread_todos_changed` — see `web_chat::event_bus`); these RPCs are the
//! one-shot "what is it right now" read a client makes when it opens a thread
//! that already had one in flight. `state_get` reads both plus the agent's
//! notes in one call, with the run-state block they render to.

use super::support::{envelope, workspace_dir};
use crate::agent::goals::{goal_store, goal_to_value};
use crate::core::Outcome;
use crate::threads::ApiEnvelope;

/// Request for [`goal_get`] / [`todos_get`] / [`state_get`]: the thread to read.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadLiveStateRequest {
    pub thread_id: String,
}

/// Response for [`goal_get`]: the thread's current goal, or `None` when the
/// thread has none. Mirrors the `goal` field on `ThreadGoalUpdated` /
/// `thread_goal_updated` — a raw `Value` to keep the RPC contract independent
/// of the harness tool's internal goal type.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadGoalGetResponse {
    pub goal: Option<serde_json::Value>,
}

/// Response for [`todos_get`]: the thread's current todo list. Empty when the
/// thread has never written one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTodosGetResponse {
    pub todos: Vec<tinyagents_harness_tools::todos::TodoItem>,
}

/// Response for [`state_get`]: what the agent's run-state block is built
/// from (goal, plan and the agent's notes) and the block a turn would start
/// with. For debugging and inspection; the UI keeps to the goal chip.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadStateGetResponse {
    pub state: tinyagents_harness_tools::state_block::RunState,
    pub block: Option<String>,
}

/// `threads.goal_get` — read a thread's current goal without waiting for the
/// next `thread_goal_updated` push (e.g. hydrating the goal chip when the
/// user reopens a thread that already had a goal in flight).
pub async fn goal_get(
    request: ThreadLiveStateRequest,
) -> Result<Outcome<ApiEnvelope<ThreadGoalGetResponse>>, String> {
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let context = crate::core::runtime::DispatchCtx::ambient();
    let goal = match goal_store(&dir, &context).get(thread_id).await {
        Ok(goal) => goal.as_ref().map(goal_to_value),
        Err(error) => {
            log::debug!("[threads] goal_get thread_id={thread_id} failed: {error}");
            None
        }
    };
    Ok(envelope(ThreadGoalGetResponse { goal }, None, None))
}

/// `threads.todos_get` — read a thread's current session todo list without
/// waiting for the next `thread_todos_changed` push.
pub async fn todos_get(
    request: ThreadLiveStateRequest,
) -> Result<Outcome<ApiEnvelope<ThreadTodosGetResponse>>, String> {
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let store = crate::storage::sessions::stores_or_ephemeral(&dir).kv;
    let todos = match tinyagents_harness_tools::todos::store::list(&store, thread_id).await {
        Ok(snapshot) => snapshot.items,
        Err(e) => {
            log::debug!("[threads] todos_get thread_id={thread_id} list failed: {e}");
            Vec::new()
        }
    };
    Ok(envelope(ThreadTodosGetResponse { todos }, None, None))
}

/// `threads.state_get` — read a thread's saved run state in one call: the
/// goal, the plan, the agent's notes (and its last finished task's), plus the
/// run-state block a turn would start with. The per-run facts (failures,
/// files written) exist only inside a running turn and are not included.
pub async fn state_get(
    request: ThreadLiveStateRequest,
) -> Result<Outcome<ApiEnvelope<ThreadStateGetResponse>>, String> {
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let store = crate::storage::sessions::stores_or_ephemeral(&dir).kv;
    // Each part is read on its own: one the store cannot return is left out
    // (and logged by TinyAgents), the rest still comes back.
    let state = tinyagents_harness_tools::state_block::RunState::load(&store, thread_id).await;
    log::debug!(
        "[threads] state_get thread_id={thread_id} goal={} steps={} notes={}",
        state.goal.is_some(),
        state.todos.len(),
        state.notes.is_some()
    );
    let block = state.render();
    Ok(envelope(
        ThreadStateGetResponse { state, block },
        None,
        None,
    ))
}

#[cfg(test)]
#[path = "live_state_tests.rs"]
mod tests;
