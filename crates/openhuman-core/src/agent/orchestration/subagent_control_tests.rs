use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 2);
    assert_eq!(schema_for("subagent_cancel").namespace, "subagent");
    assert_eq!(schema_for("subagent_cancel").function, "cancel");
    assert_eq!(schema_for("subagent_steer").namespace, "subagent");
    assert_eq!(schema_for("subagent_steer").function, "steer");
}

#[test]
fn require_str_rejects_blank_and_missing() {
    let mut params = Map::new();
    assert!(require_str(&params, "taskId").is_err());
    params.insert("taskId".into(), json!("   "));
    assert!(require_str(&params, "taskId").is_err());
    params.insert("taskId".into(), json!("sub-1"));
    assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
    // Whitespace-padded ids are trimmed so they match the registry key.
    params.insert("taskId".into(), json!("  sub-1  "));
    assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
}

#[tokio::test]
async fn cancel_unknown_task_is_a_noop_false() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let mut params = Map::new();
    params.insert("taskId".into(), json!("sub-does-not-exist"));
    let out = handle_subagent_cancel(DispatchCtx::process_default(), params)
        .await
        .expect("handler ok");
    // Outcome wraps the payload under `data`.
    let cancelled = out
        .get("data")
        .and_then(|d| d.get("cancelled"))
        .or_else(|| out.get("cancelled"))
        .and_then(Value::as_bool);
    assert_eq!(cancelled, Some(false));
    // Nothing known about the task: the card must not claim it succeeded.
    assert_eq!(field(&out, "outcome"), Some(json!("unknown")));
}

fn field(out: &Value, name: &str) -> Option<Value> {
    out.get("data")
        .and_then(|d| d.get(name))
        .or_else(|| out.get(name))
        .cloned()
}

/// A late Cancel on a run that already finished answers with how it ended and
/// leaves its durable session alone, instead of rewriting it as "cancelled by
/// user". A failed run must come back `failed`, never as a success.
#[tokio::test]
async fn cancel_of_a_finished_run_reports_its_outcome_and_rewrites_nothing() {
    use crate::agent::orchestration::running_subagents::{register, status_channel};
    use crate::agent::orchestration::subagent_sessions::SubagentSessionStore;
    use std::sync::Arc;
    use tinyagents_harness::run_queue::RunQueue;
    use tinyagents_orchestration::subagent::DetachedSubagentStatus;

    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("tempdir");
    // Snapshot the durable session store before and after: a real cancel
    // would `mark_failed` the session and rewrite it.
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let seeded: Vec<crate::agent::orchestration::subagent_sessions::DurableSubagentSession> =
        serde_json::from_value(json!([{
            "subagentSessionId": "subsess-sub-rpc-done",
            "parentSession": "session-rpc",
            "parentThreadId": "thread-rpc",
            "workerThreadId": null,
            "agentId": "agent_memory",
            "displayName": null,
            "model": null,
            "sandboxMode": "workspace",
            "actionRoot": null,
            "taskKey": "task",
            "taskTitle": "Task",
            "currentTaskId": "sub-rpc-done",
            "status": "running",
            "reusable": true,
            "latestHistory": null,
            "latestError": null,
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z",
            "lastUsedAt": "2026-01-01T00:00:00Z"
        }]))
        .expect("session fixture");
    store.save(&seeded).expect("seed store");
    let before = serde_json::to_string(&store.load().expect("load seeded")).expect("encode");
    for (task_id, status, outcome) in [
        (
            "sub-rpc-done",
            DetachedSubagentStatus::Completed {
                output: "ok".into(),
                iterations: 1,
            },
            "completed",
        ),
        (
            "sub-rpc-failed",
            DetachedSubagentStatus::Failed {
                error: "boom".into(),
            },
            "failed",
        ),
    ] {
        let (tx, rx) = status_channel();
        register(
            &crate::core::runtime::DispatchCtx::ambient(),
            task_id.into(),
            "agent_memory".into(),
            "session-rpc".into(),
            None,
            Some(format!("subsess-{task_id}")),
            workspace.path().to_path_buf(),
            Some("thread-rpc".into()),
            Arc::new(RunQueue::new()),
            tokio::spawn(async {}).abort_handle(),
            rx,
        )
        .expect("fixture registration admitted");
        tx.send(status).expect("status channel open");

        let mut params = Map::new();
        params.insert("taskId".into(), json!(task_id));
        let out = handle_subagent_cancel(DispatchCtx::process_default(), params)
            .await
            .expect("handler ok");
        assert_eq!(field(&out, "cancelled"), Some(json!(false)), "{task_id}");
        assert_eq!(field(&out, "outcome"), Some(json!(outcome)), "{task_id}");
    }
    assert_eq!(
        serde_json::to_string(&store.load().expect("store still readable")).expect("encode"),
        before,
        "a finished run's session must not be rewritten as cancelled"
    );
}

#[tokio::test]
async fn steer_unknown_task_is_a_noop_false() {
    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let mut params = Map::new();
    params.insert("taskId".into(), json!("sub-does-not-exist"));
    params.insert("message".into(), json!("redirect"));
    let out = handle_subagent_steer(DispatchCtx::process_default(), params)
        .await
        .expect("handler ok");
    let data = out.get("data").unwrap_or(&out);
    assert_eq!(data.get("steered").and_then(Value::as_bool), Some(false));
    assert_eq!(data.get("reason").and_then(Value::as_str), Some("unknown"));
}
