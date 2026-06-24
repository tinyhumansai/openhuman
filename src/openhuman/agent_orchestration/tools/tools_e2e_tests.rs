use super::{
    ArchetypeDelegationTool, SkillDelegationTool, SpawnSubagentTool, SpawnWorkerThreadTool,
};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::{with_parent_context, ParentExecutionContext};
use crate::openhuman::context::prompt::{ConnectedIntegration, ToolCallFormat};
use crate::openhuman::inference::provider::{ChatMessage, ChatRequest, ChatResponse, Provider};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::memory_conversations as conversations;
use crate::openhuman::tools::Tool;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

const SPAWN_SUBAGENT_CANARY: &str = "tool-e2e-spawn-subagent-canary";
const ARCHETYPE_DELEGATION_CANARY: &str = "tool-e2e-archetype-delegation-canary";
const SKILL_DELEGATION_CANARY: &str = "tool-e2e-skill-delegation-canary";
const WORKER_THREAD_CANARY: &str = "tool-e2e-worker-thread-canary";

#[tokio::test]
async fn spawn_subagent_tool_runs_child_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedProvider::new(vec![(
        SPAWN_SUBAGENT_CANARY,
        "spawn-subagent-child-answer",
    )]));

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            SpawnSubagentTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": format!("Investigate {SPAWN_SUBAGENT_CANARY}"),
                    "context": "parent supplied context",
                    "model": "test-model",
                    "blocking": true
                }))
                .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "spawn-subagent-child-answer");
    assert!(provider.saw(SPAWN_SUBAGENT_CANARY));
    assert!(provider.saw("parent supplied context"));
}

#[tokio::test]
async fn archetype_delegation_tool_runs_child_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedProvider::new(vec![(
        ARCHETYPE_DELEGATION_CANARY,
        "archetype-delegation-child-answer",
    )]));
    let tool = ArchetypeDelegationTool {
        tool_name: "delegate_researcher".to_string(),
        agent_id: "researcher".to_string(),
        tool_description: "Delegate research work.".to_string(),
    };

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            tool.execute(json!({
                "prompt": format!("Research {ARCHETYPE_DELEGATION_CANARY}"),
                "model": "test-model"
            }))
            .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "archetype-delegation-child-answer");
    assert!(provider.saw(ARCHETYPE_DELEGATION_CANARY));
}

#[tokio::test]
async fn skill_delegation_tool_runs_integrations_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedProvider::new(vec![(
        SKILL_DELEGATION_CANARY,
        "skill-delegation-child-answer",
    )]));
    let tool = SkillDelegationTool::for_connected(vec![(
        "gmail".to_string(),
        "Email access.".to_string(),
    )])
    .expect("delegation tool");

    let result = with_parent_context(
        parent_context(
            workspace.path(),
            provider.clone(),
            vec![ConnectedIntegration {
                toolkit: "gmail".to_string(),
                description: "Email access.".to_string(),
                tools: Vec::new(),
                gated_tools: Vec::new(),
                connected: true,
                connections: Vec::new(),
                non_active_status: None,
            }],
        ),
        async {
            tool.execute(json!({
                "toolkit": "gmail",
                "prompt": format!("Summarize inbox state for {SKILL_DELEGATION_CANARY}"),
                "model": "test-model"
            }))
            .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "skill-delegation-child-answer");
    assert!(provider.saw(SKILL_DELEGATION_CANARY));
    assert!(provider.saw("gmail"));
}

#[tokio::test]
async fn spawn_worker_thread_tool_persists_worker_thread_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedProvider::new(vec![(
        WORKER_THREAD_CANARY,
        "worker-thread-child-answer",
    )]));

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            SpawnWorkerThreadTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": format!("Handle long task {WORKER_THREAD_CANARY}"),
                    "task_title": "Long delegated task",
                    "model": "test-model"
                }))
                .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("[worker_thread_ref]"));
    assert!(result.output().contains("\"status\":\"completed\""));
    assert!(provider.saw(WORKER_THREAD_CANARY));

    let threads =
        conversations::list_threads(workspace.path().to_path_buf()).expect("worker threads");
    let worker = threads
        .iter()
        .find(|thread| thread.labels.contains(&"tasks".to_string()))
        .expect("worker thread was persisted");
    assert_eq!(worker.title, "Long delegated task");

    let messages = conversations::get_messages(workspace.path().to_path_buf(), &worker.id)
        .expect("worker messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sender, "user");
    assert!(messages[0].content.contains(WORKER_THREAD_CANARY));
    assert_eq!(messages[1].sender, "agent");
    assert_eq!(messages[1].content, "worker-thread-child-answer");
}

fn parent_context(
    workspace_dir: &Path,
    provider: Arc<dyn Provider>,
    connected_integrations: Vec<ConnectedIntegration>,
) -> ParentExecutionContext {
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["researcher".to_string(), "integrations_agent".to_string()]
            .into_iter()
            .collect(),
        provider,
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: workspace_dir.to_path_buf(),
        memory: Arc::new(NoopMemory),
        agent_config: Default::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "tools-e2e-session".into(),
        channel: "test".into(),
        connected_integrations,
        tool_call_format: ToolCallFormat::Native,
        session_key: "tools-e2e".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

struct ScriptedProvider {
    responses: Vec<(&'static str, &'static str)>,
    seen: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<(&'static str, &'static str)>) -> Self {
        Self {
            responses,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn saw(&self, needle: &str) -> bool {
        self.seen
            .lock()
            .iter()
            .any(|payload| payload.contains(needle))
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.seen.lock().push(message.to_string());
        Ok("ok".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let flattened = flatten_messages(request.messages);
        self.seen.lock().push(flattened.clone());
        for (needle, answer) in &self.responses {
            if flattened.contains(needle) {
                return Ok(ChatResponse {
                    text: Some((*answer).to_string()),
                    tool_calls: Vec::new(),
                    usage: None,
                    reasoning_content: None,
                });
            }
        }
        anyhow::bail!("unexpected provider request: {flattened}");
    }
}

fn flatten_messages(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|message| format!("{}:{}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _value: &str,
        _category: MemoryCategory,
        _source: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _source: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "noop"
    }
}
