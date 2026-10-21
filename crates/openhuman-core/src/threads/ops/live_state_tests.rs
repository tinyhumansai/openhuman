//! Behavior tests for `threads::ops::live_state`.
//!
//! `goal_get`/`todos_get` resolve their workspace through
//! `Config::load_or_init()` (the same process-global config every RPC
//! handler reads), so — like the other `OPENHUMAN_WORKSPACE`-dependent config
//! tests — these serialize on `crate::config::TEST_ENV_LOCK` and point that
//! env var at a fresh tempdir for the duration of the test.

use super::*;
use crate::config::TEST_ENV_LOCK;
use tinyagents_harness_tools::todos::{TodoItem, TodoStatus};

#[test]
fn thread_live_state_request_parses_thread_id() {
    let parsed: ThreadLiveStateRequest =
        serde_json::from_value(serde_json::json!({ "thread_id": "thread-1" })).unwrap();
    assert_eq!(parsed.thread_id, "thread-1");
}

struct WorkspaceGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    _tmp: tempfile::TempDir,
}

impl WorkspaceGuard {
    async fn new() -> Self {
        let lock = TEST_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        Self {
            _lock: lock,
            _tmp: tmp,
        }
    }
}

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

/// `threads.goal_get` returns `{ goal: null }` for a thread with no goal, and
/// the full goal payload — the same shape `ThreadGoalUpdated` carries — once
/// one is set.
#[tokio::test]
async fn goal_get_reads_back_a_stored_goal() {
    let _ws = WorkspaceGuard::new().await;

    let empty = goal_get(ThreadLiveStateRequest {
        thread_id: "thread-goal-live".to_string(),
    })
    .await
    .unwrap();
    let empty_json = empty.into_cli_compatible_json().unwrap();
    assert!(empty_json["data"]["goal"].is_null(), "{empty_json}");

    let dir = crate::config::Config::load_or_init()
        .await
        .unwrap()
        .workspace_dir;
    tinyagents_harness_tools::goals::store::set(
        &crate::storage::sessions::stores_or_ephemeral(&dir).kv,
        "thread-goal-live",
        "ship it",
        Some(1000),
    )
    .await
    .unwrap();

    let filled = goal_get(ThreadLiveStateRequest {
        thread_id: "thread-goal-live".to_string(),
    })
    .await
    .unwrap();
    let filled_json = filled.into_cli_compatible_json().unwrap();
    let goal = &filled_json["data"]["goal"];
    assert_eq!(goal["objective"], "ship it", "{filled_json}");
    assert_eq!(goal["status"], "active");
}

/// `threads.todos_get` reads back what a `TodoTool` call (thread-id-keyed)
/// wrote for the same thread.
#[tokio::test]
async fn todos_get_reads_back_what_the_todo_tool_wrote() {
    let _ws = WorkspaceGuard::new().await;
    let dir = crate::config::Config::load_or_init()
        .await
        .unwrap()
        .workspace_dir;

    let empty = todos_get(ThreadLiveStateRequest {
        thread_id: "thread-todos-live".to_string(),
    })
    .await
    .unwrap();
    let empty_json = empty.into_cli_compatible_json().unwrap();
    assert!(empty_json["data"]["todos"].as_array().unwrap().is_empty());

    let store = crate::storage::sessions::stores_or_ephemeral(&dir).kv;
    tinyagents_harness_tools::todos::store::replace(
        &store,
        "thread-todos-live",
        vec![TodoItem::with_status("write tests", TodoStatus::InProgress)],
    )
    .await
    .unwrap();

    let filled = todos_get(ThreadLiveStateRequest {
        thread_id: "thread-todos-live".to_string(),
    })
    .await
    .unwrap();
    let filled_json = filled.into_cli_compatible_json().unwrap();
    let todos = filled_json["data"]["todos"]
        .as_array()
        .unwrap_or_else(|| panic!("missing todos in {filled_json}"));
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0]["content"], "write tests");
}
