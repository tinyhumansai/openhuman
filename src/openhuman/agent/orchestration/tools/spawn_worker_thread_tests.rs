use super::*;
use crate::openhuman::agent::harness::fork_context::with_parent_context;
use crate::openhuman::agent::harness::ParentExecutionContext;
use crate::openhuman::memory::conversations::CreateConversationThread;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

struct MockMemory;
#[async_trait]
impl crate::openhuman::memory::Memory for MockMemory {
    async fn store(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: crate::openhuman::memory::MemoryCategory,
        _: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _: &str,
        _: usize,
        _: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _: Option<&str>,
        _: Option<&crate::openhuman::memory::MemoryCategory>,
        _: Option<&str>,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _: &str, _: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(vec![])
    }
    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "mock"
    }
}

fn test_parent_ctx(workspace_dir: PathBuf) -> ParentExecutionContext {
    let model: Arc<dyn tinyinference::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "done",
        ]));
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::new(),
        session_id: "test".into(),
        session_key: "test".into(),
        session_parent_prefix: None,
        model_name: "test".into(),
        temperature: 0.4,
        workspace_dir,
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        memory: Arc::new(MockMemory),
        channel: "test".into(),
        all_tools: Arc::new(vec![]),
        all_tool_specs: Arc::new(vec![]),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        workflows: Arc::new(vec![]),
        memory_context: std::sync::Arc::new(None),
        connected_integrations: vec![],
        on_progress: None,
        run_queue: None,
        agent_config: crate::openhuman::config::AgentConfig::default(),
        tool_call_format: crate::openhuman::agent::context::prompt::ToolCallFormat::Native,
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

    crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
        thread_id.to_string(),
        async {
            let parent = test_parent_ctx(temp.path().to_path_buf());
            with_parent_context(parent, async {
                let tool = SpawnWorkerThreadTool::new();
                let result = tool
                    .execute(json!({
                        "agent_id": "researcher",
                        "prompt": "do it",
                        "task_title": "Task"
                    }))
                    .await
                    .unwrap();

                assert!(result.is_error);
                assert!(result
                    .output()
                    .contains("cannot spawn other worker threads"));
            })
            .await;
        },
    )
    .await;
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

    crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
        thread_id.to_string(),
        async {
            let parent = test_parent_ctx(temp.path().to_path_buf());
            with_parent_context(parent, async {
                let tool = SpawnWorkerThreadTool::new();
                let result = tool
                    .execute(json!({
                        "agent_id": "researcher",
                        "prompt": "do it",
                        "task_title": "Task"
                    }))
                    .await
                    .unwrap();

                assert!(result.is_error);
                assert!(result
                    .output()
                    .contains("cannot spawn other worker threads"));
            })
            .await;
        },
    )
    .await;
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
                "agent_id": "researcher",
                "prompt": "do it",
                "task_title": "Task"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains(
            "spawn_worker_thread: agent 'researcher' is not in parent agent 'orchestrator' subagents.allowlist"
        ));
    })
    .await;
}
