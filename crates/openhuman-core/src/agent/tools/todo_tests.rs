use super::*;
use crate::agent::todos::ops::{TodoItem, TodoStatus};
use serde_json::{json, Value};

/// Serialize tests that share the process-global scratch store. Same lock
/// as `todos::ops` — otherwise the two test modules race under `cargo test`'s
/// thread pool.
fn scratch_lock() -> tokio::sync::MutexGuard<'static, ()> {
    crate::agent::todos::ops::scratch_test_lock()
}

async fn scratch_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    crate::agent::todos::ops::scratch_test_lock_async().await
}

/// A fresh on-disk workspace root for one test's `FileStore`-backed todo
/// list. Each test gets its own tempdir so tests never see each other's
/// persisted lists.
fn test_workspace() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("openhuman-todo-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create test workspace");
    dir
}

async fn reset_scratch(workspace_dir: &std::path::Path) {
    crate::agent::todos::ops::clear(workspace_dir, &TodoScope::Scratch)
        .await
        .expect("clear scratch");
}

fn payload(result: &ToolResult) -> Value {
    serde_json::from_str(&result.output()).expect("json payload")
}

#[tokio::test]
async fn a_write_replaces_the_whole_list_and_a_read_returns_it() {
    let _guard = scratch_lock_async().await;
    let workspace_dir = test_workspace();
    reset_scratch(&workspace_dir).await;
    let tool = TodoTool::new(workspace_dir.clone());

    let written = tool
        .execute(json!({ "todos": [
            { "content": "Write tests", "status": "in_progress" },
            { "content": "Ship it", "status": "pending" }
        ] }))
        .await
        .unwrap();
    assert!(!written.is_error, "{}", written.output());
    let p = payload(&written);
    assert_eq!(p["todos"].as_array().unwrap().len(), 2);
    assert_eq!(p["todos"][0]["status"], "in_progress");
    assert_eq!(p["todos"][1]["status"], "pending");
    let markdown = p["markdown"].as_str().unwrap();
    assert!(markdown.contains("[~] Write tests"), "{markdown}");
    assert!(markdown.contains("[ ] Ship it"), "{markdown}");

    // Omitting `todos` reads the list back.
    let read = tool.execute(json!({})).await.unwrap();
    assert_eq!(payload(&read)["todos"].as_array().unwrap().len(), 2);

    // The next write is the whole list again, not a patch.
    let rewritten = tool
        .execute(json!({ "todos": [
            { "content": "Write tests", "status": "completed" }
        ] }))
        .await
        .unwrap();
    let p = payload(&rewritten);
    assert_eq!(p["todos"].as_array().unwrap().len(), 1);
    assert_eq!(p["todos"][0]["status"], "completed");
    assert!(p["markdown"].as_str().unwrap().contains("[x] Write tests"));

    // An empty list clears it.
    let cleared = tool.execute(json!({ "todos": [] })).await.unwrap();
    assert!(payload(&cleared)["todos"].as_array().unwrap().is_empty());
    reset_scratch(&workspace_dir).await;
}

/// Bad input is a tool error the model can correct, never an `Err`: a
/// dispatch `Err` is fatal to the whole run in the harness, and a turn died
/// exactly that way when a model sent the retired `{"cards": …}` shape.
#[tokio::test]
async fn bad_input_is_a_tool_error_not_a_harness_error() {
    let tool = TodoTool::new(test_workspace());
    for (args, expect) in [
        (
            json!({ "todos": [{ "content": "  ", "status": "pending" }] }),
            "content",
        ),
        (
            // The exact phrasing belongs to TinyAgents; assert only that the
            // rejection names the field the model got wrong.
            json!({ "todos": [{ "content": "x", "status": "someday" }] }),
            "status",
        ),
        (json!({ "todos": "not a list" }), "invalid `todos`"),
        (
            json!({ "cards": [{ "content": "x", "status": "todo" }] }),
            "pass `todos`",
        ),
    ] {
        let result = tool
            .execute(args.clone())
            .await
            .expect("never an Err: {args}");
        assert!(result.is_error, "{args}");
        assert!(
            result.output().contains(expect),
            "{args}: {}",
            result.output()
        );
    }
}

/// The orchestrator's list is its thread's list — keyed by the chat thread
/// id, not `ParentExecutionContext::session_id` (which for the web channel is
/// the `{client_id,thread_id}` JSON blob and would otherwise scatter one
/// thread's todos across every reconnect). It used to be routed to one
/// app-wide `orchestrator-tasks` board that nothing rendered, so the items
/// the model wrote never showed up in the thread the user was in.
#[test]
fn every_agent_binds_to_its_own_thread() {
    struct ThreadContext(&'static str);
    impl ToolRunContext for ThreadContext {
        fn thread_id(&self) -> Option<&str> {
            Some(self.0)
        }
    }
    let parent = ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["done"]),
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: std::path::PathBuf::from("/tmp/openhuman-todo-parent"),
        workspace_descriptor: None,
        agent_config: crate::config::AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "orchestrator_thread-live".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: crate::agent::prompts::ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    };

    assert_eq!(
        current_scope(Some(&parent), Some(&ThreadContext("thread-live"))).session_id(),
        Some("thread-live"),
        "the thread id wins over the parent's legacy session_id"
    );
    assert_eq!(
        current_scope(None, Some(&ThreadContext("thread-live"))).session_id(),
        Some("thread-live"),
        "a thread-only caller keys on the thread"
    );
    assert_eq!(
        current_scope(Some(&parent), None).session_id(),
        Some("orchestrator_thread-live"),
        "no thread id at all falls back to the legacy parent session_id"
    );
    assert_eq!(current_scope(None, None), TodoScope::Scratch);
}

/// A list left under the pre-rekey `session_id` key is picked up once by the
/// new thread-id key, instead of silently disappearing when this file's
/// scope switched from `session_id`-first to `thread_id`-first.
#[tokio::test]
async fn a_legacy_session_keyed_list_is_migrated_forward_once() {
    let workspace_dir = test_workspace();
    struct ThreadContext(&'static str);
    impl ToolRunContext for ThreadContext {
        fn thread_id(&self) -> Option<&str> {
            Some(self.0)
        }
    }
    let parent = ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["done"]),
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace_dir.clone(),
        workspace_descriptor: None,
        agent_config: crate::config::AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "client-1|thread-legacy".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: crate::agent::prompts::ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    };

    // Seed the legacy session-keyed list directly through the store, as if a
    // pre-rekey process had written it.
    let legacy_scope = TodoScope::Session {
        id: parent.session_id.clone(),
    };
    let item = TodoItem::with_status("carried over", TodoStatus::InProgress);
    crate::agent::todos::ops::replace(&workspace_dir, &legacy_scope, vec![item])
        .await
        .unwrap();

    let tool = TodoTool::new(workspace_dir.clone());
    let read = tool
        .execute_with_parent_context(
            json!({}),
            Some(parent),
            Some(&ThreadContext("thread-legacy")),
        )
        .await
        .unwrap();
    let p = payload(&read);
    let todos = p["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 1, "{p}");
    assert_eq!(todos[0]["content"], "carried over");

    // The migrated list now lives under the thread-id key too.
    let thread_scope = TodoScope::Session {
        id: "thread-legacy".into(),
    };
    let migrated = crate::agent::todos::ops::list(&workspace_dir, &thread_scope)
        .await
        .unwrap();
    assert_eq!(migrated.items.len(), 1);
}

#[tokio::test]
async fn sessions_do_not_see_each_other_and_a_list_survives_across_turns() {
    let workspace_dir = test_workspace();
    let a = TodoScope::Session {
        id: "sess-a".into(),
    };
    let b = TodoScope::Session {
        id: "sess-b".into(),
    };
    crate::agent::todos::ops::clear(&workspace_dir, &a)
        .await
        .unwrap();
    crate::agent::todos::ops::clear(&workspace_dir, &b)
        .await
        .unwrap();

    let item = TodoItem::with_status("only in a", TodoStatus::InProgress);
    crate::agent::todos::ops::replace(&workspace_dir, &a, vec![item])
        .await
        .unwrap();

    let a_again = crate::agent::todos::ops::list(&workspace_dir, &a)
        .await
        .unwrap();
    assert_eq!(
        a_again.items.len(),
        1,
        "a later turn of the same session reads it back"
    );
    assert_eq!(a_again.thread_id, "sess-a");
    assert!(crate::agent::todos::ops::list(&workspace_dir, &b)
        .await
        .unwrap()
        .items
        .is_empty());
}
