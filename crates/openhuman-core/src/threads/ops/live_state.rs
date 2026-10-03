//! Read-back RPCs for the two thread-scoped, agent-driven surfaces a client
//! needs to hydrate on load or reconnect: the thread's goal (`agent::goals`)
//! and its session todo list (`agent::todos`). Live updates for both stream
//! separately over the web channel (`thread_goal_updated`/`thread_goal_cleared`,
//! `thread_todos_changed` — see `web_chat::event_bus`); these RPCs are the
//! one-shot "what is it right now" read a client makes when it opens a thread
//! that already had one in flight.

use super::support::{envelope, workspace_dir};
use crate::agent::goals::goal_to_value;
use crate::agent::todos::ops::{self as todos_ops, TodoScope};
use crate::core::Outcome;
use crate::threads::ApiEnvelope;

/// Request for [`goal_get`] / [`todos_get`]: the thread to read.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadLiveStateRequest {
    pub thread_id: String,
}

/// Response for [`goal_get`]: the thread's current goal, or `None` when the
/// thread has none. Mirrors the `goal` field on `ThreadGoalUpdated` /
/// `thread_goal_updated` — a raw `Value` because `ThreadGoal` is owned by
/// `tinyagents-graph`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadGoalGetResponse {
    pub goal: Option<serde_json::Value>,
}

/// Response for [`todos_get`]: the thread's current todo list. Empty when the
/// thread has never written one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTodosGetResponse {
    pub todos: Vec<crate::agent::todos::ops::TodoItem>,
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
    let goal = crate::agent::goals::runtime::load_for_thread(&dir, Some(thread_id))
        .await
        .as_ref()
        .map(goal_to_value);
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
    let scope = TodoScope::Session {
        id: thread_id.to_string(),
    };
    let todos = match todos_ops::list(&dir, &scope).await {
        Ok(snapshot) => snapshot.items,
        Err(e) => {
            log::debug!("[threads] todos_get thread_id={thread_id} list failed: {e}");
            Vec::new()
        }
    };
    Ok(envelope(ThreadTodosGetResponse { todos }, None, None))
}

#[cfg(test)]
#[path = "live_state_tests.rs"]
mod tests;
