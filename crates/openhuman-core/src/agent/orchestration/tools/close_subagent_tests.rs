use super::*;
use crate::agent::harness::fork_context::ParentExecutionContext;
use crate::agent::prompts::ToolCallFormat;
use crate::config::AgentConfig;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::tool::ToolDispatch;

#[tokio::test]
async fn missing_session_id_is_rejected() {
    let res = CloseSubagentTool::new().execute(json!({})).await.unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("subagent_session_id"));
}

#[tokio::test]
async fn rejects_session_from_different_parent_thread() {
    let workspace = tempfile::TempDir::new().expect("workspace");
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let session = seed_session(&store, "thread-b");

    let res = close_for_thread(workspace.path(), "thread-a", &session.subagent_session_id).await;

    assert!(res.is_error);
    assert!(res.output().contains("not found for this parent thread"));
    assert!(
        subagent_sessions::find_reusable(&store, &selector("thread-b"))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn closes_session_owned_by_current_parent_thread() {
    let workspace = tempfile::TempDir::new().expect("workspace");
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let session = seed_session(&store, "thread-a");

    let res = close_for_thread(workspace.path(), "thread-a", &session.subagent_session_id).await;

    assert!(!res.is_error, "{}", res.output());
    assert!(res.output().contains("closed=true"));
    assert!(
        subagent_sessions::find_reusable(&store, &selector("thread-a"))
            .unwrap()
            .is_none()
    );
}

async fn close_for_thread(
    workspace: &Path,
    thread_id: &str,
    subagent_session_id: &str,
) -> tinytools::ToolResult {
    let parent = parent_context(workspace);
    let run = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_parent(parent)
        .into_tinyagents(RunConfig::new("close-subagent-test").with_thread(thread_id));
    CloseSubagentDispatch::new(Arc::new(CloseSubagentTool::new()))
        .execute(
            &(),
            tinyagents_harness::ids::CallId::new("close-subagent-test"),
            json!({ "subagent_session_id": subagent_session_id }),
            tinytools::ToolCallOptions::default(),
            &run,
        )
        .await
        .expect("close dispatch")
}

fn seed_session(
    store: &SubagentSessionStore,
    parent_thread_id: &str,
) -> subagent_sessions::DurableSubagentSession {
    subagent_sessions::upsert_running(
        store,
        subagent_sessions::SubagentSessionUpsert {
            selector: selector(parent_thread_id),
            display_name: Some("Researcher".into()),
            task_title: "Task".into(),
            worker_thread_id: Some("worker-1".into()),
            task_id: "sub-1".into(),
        },
        None,
    )
    .unwrap()
}

fn selector(parent_thread_id: &str) -> subagent_sessions::SubagentSessionSelector {
    subagent_sessions::SubagentSessionSelector {
        parent_session: "parent-session".into(),
        parent_thread_id: Some(parent_thread_id.into()),
        agent_id: "researcher".into(),
        model: None,
        sandbox_mode: "workspace".into(),
        action_root: None,
        task_key: "task".into(),
    }
}

fn parent_context(workspace_dir: &Path) -> ParentExecutionContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    ParentExecutionContext {
        runtime_config: None,
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace_dir.to_path_buf(),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "parent-session".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}
