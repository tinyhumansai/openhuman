use super::*;
use serde_json::Value;

/// Serialize tests that share the process-global scratch store with
/// `todos::ops` tests. Same lock — otherwise the two test modules race
/// under `cargo test`'s thread pool.
fn scratch_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::threads::todos::ops::scratch_test_lock()
}

async fn reset_scratch() {
    crate::openhuman::threads::todos::ops::clear(&BoardLocation::Scratch)
        .await
        .expect("clear scratch");
}

#[tokio::test]
async fn add_then_list_round_trips_via_scratch() {
    let _guard = scratch_lock();
    reset_scratch().await;
    let tool = TodoTool::new();
    let added = tool
        .execute(json!({ "op": "add", "content": "Write tests" }))
        .await
        .unwrap();
    assert!(!added.is_error, "{}", added.output());
    let payload: Value = serde_json::from_str(&added.output()).unwrap();
    let cards = payload["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    let id = cards[0]["id"].as_str().unwrap().to_string();
    assert!(payload["markdown"]
        .as_str()
        .unwrap()
        .contains("[ ] Write tests"));

    let listed = tool.execute(json!({ "op": "list" })).await.unwrap();
    let listed_payload: Value = serde_json::from_str(&listed.output()).unwrap();
    assert_eq!(listed_payload["cards"].as_array().unwrap().len(), 1);

    let done = tool
        .execute(json!({ "op": "update_status", "id": id, "status": "done" }))
        .await
        .unwrap();
    let done_payload: Value = serde_json::from_str(&done.output()).unwrap();
    assert!(done_payload["markdown"]
        .as_str()
        .unwrap()
        .contains("[x] Write tests"));
    reset_scratch().await;
}

#[tokio::test]
async fn unknown_op_returns_error() {
    let tool = TodoTool::new();
    let result = tool.execute(json!({ "op": "frobnicate" })).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("unknown op"));
}

#[tokio::test]
async fn add_requires_content() {
    let tool = TodoTool::new();
    let err = tool.execute(json!({ "op": "add" })).await.unwrap_err();
    assert!(err.to_string().contains("content"));
}

#[test]
fn description_carries_planning_guidance() {
    // The `todo` tool steers the live orchestrator purely through its static
    // (prompt-cache-stable) schema description — there is no per-thread prompt
    // injection. Lock in the behavioural contract so the guidance can't be
    // silently dropped: when-to-use, single-in_progress discipline, and the
    // "bound to the current thread, don't pass a thread id" rule.
    let tool = TodoTool::new();
    let desc = tool.description();
    assert!(desc.contains("3+ steps"), "missing when-to-use guidance");
    assert!(
        desc.contains("Keep one `in_progress`"),
        "missing single-in_progress discipline"
    );
    assert!(
        desc.contains("do not pass a thread id"),
        "missing explicit 'do not pass a thread id' note"
    );
}

#[tokio::test]
async fn edit_rejects_unknown_id() {
    let _guard = scratch_lock();
    reset_scratch().await;
    let tool = TodoTool::new();
    let result = tool
        .execute(json!({ "op": "edit", "id": "task-missing", "content": "x" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not found"));
    reset_scratch().await;
}

#[tokio::test]
async fn replace_accepts_full_card_list() {
    let _guard = scratch_lock();
    reset_scratch().await;
    let tool = TodoTool::new();
    let result = tool
        .execute(json!({
            "op": "replace",
            "cards": [
                {
                    "id": "",
                    "title": "Alpha",
                    "status": "todo",
                    "order": 0,
                    "updated_at": ""
                },
                {
                    "id": "",
                    "title": "Beta",
                    "status": "in_progress",
                    "order": 1,
                    "updated_at": ""
                }
            ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let payload: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(payload["cards"].as_array().unwrap().len(), 2);
    reset_scratch().await;
}
