use super::*;
use crate::agent::harness::fork_context::with_parent_context;
use crate::agent::harness::ParentExecutionContext;
use crate::threads::store::CreateConversationThread;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::tool::ToolDispatch;

fn test_parent_ctx(workspace_dir: PathBuf) -> ParentExecutionContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "done",
        ]));
    ParentExecutionContext {
        runtime_config: None,
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        session_id: "test".into(),
        session_key: "test".into(),
        session_parent_prefix: None,
        model_name: "test".into(),
        temperature: 0.4,
        workspace_dir,
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        channel: "test".into(),
        all_tools: Arc::new(vec![]),
        all_tool_specs: Arc::new(vec![]),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        workflows: Arc::new(vec![]),
        memory_context: std::sync::Arc::new(None),
        connected_integrations: vec![],
        on_progress: None,
        run_queue: None,
        agent_config: crate::config::AgentConfig::default(),
        tool_call_format: crate::agent::prompts::ToolCallFormat::Native,
    }
}

#[tokio::test]
async fn rejects_if_already_worker_thread() {
    let temp = TempDir::new().unwrap();
    let thread_id = "worker-123";
    conversations::ensure_thread(
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: "Worker".into(),
            created_at: "now".into(),
            parent_thread_id: None,
            labels: Some(vec!["tasks".to_string()]),
            personality_id: None,
        },
    )
    .unwrap();

    let result = spawn_from_thread(temp.path(), thread_id).await;

    assert!(result.is_error);
    assert!(result
        .output()
        .contains("cannot spawn other worker threads"));
}

#[tokio::test]
async fn rejects_if_has_parent_thread_id() {
    let temp = TempDir::new().unwrap();
    let thread_id = "sub-123";
    conversations::ensure_thread(
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: "Sub".into(),
            created_at: "now".into(),
            parent_thread_id: Some("parent".into()),
            labels: None,
            personality_id: None,
        },
    )
    .unwrap();

    let result = spawn_from_thread(temp.path(), thread_id).await;

    assert!(result.is_error);
    assert!(result
        .output()
        .contains("cannot spawn other worker threads"));
}

async fn spawn_from_thread(workspace: &std::path::Path, thread_id: &str) -> tinytools::ToolResult {
    let parent = test_parent_ctx(workspace.to_path_buf());
    let run = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_parent(parent)
        .into_tinyagents(RunConfig::new("worker-depth-test").with_thread(thread_id));
    SpawnWorkerThreadDispatch::new(Arc::new(SpawnWorkerThreadTool::new()))
        .execute(
            &(),
            tinyagents_harness::ids::CallId::new("spawn-worker-thread-test"),
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "do it",
                "task_title": "Task"
            }),
            tinytools::ToolCallOptions::default(),
            &run,
        )
        .await
        .expect("worker dispatch")
}

#[tokio::test]
async fn rejects_agent_outside_parent_allowlist() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let temp = TempDir::new().unwrap();
    let parent = test_parent_ctx(temp.path().to_path_buf());

    with_parent_context(parent, async {
        let tool = SpawnWorkerThreadTool::new();
        let result = tool
            .execute(json!({
                "agent_id": "task_manager_agent",
                "prompt": "do it",
                "task_title": "Task"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains(
            "spawn_worker_thread: agent 'task_manager_agent' is not in parent agent 'orchestrator' subagents.allowlist"
        ));
    })
    .await;
}
