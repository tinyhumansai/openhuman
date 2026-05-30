use super::*;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::config::AgentConfig;
use crate::openhuman::context::prompt::ToolCallFormat;
use crate::openhuman::inference::provider::traits::ProviderCapabilities;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::tools::{Tool, ToolSpec};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::time::{sleep, Duration};

#[derive(Default)]
struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
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
        _session_id: Option<&str>,
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

fn parent_context(provider: Arc<dyn Provider>) -> ParentExecutionContext {
    ParentExecutionContext {
        provider,
        all_tools: Arc::new(Vec::<Box<dyn Tool>>::new()),
        all_tool_specs: Arc::new(Vec::<ToolSpec>::new()),
        model_name: "test-model".to_string(),
        temperature: 0.2,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(NoopMemory),
        agent_config: AgentConfig::default(),
        skills: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "orchestrator-session".to_string(),
        channel: "test".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_orchestrator".to_string(),
        session_parent_prefix: None,
        on_progress: None,
    }
}

fn text_response(text: impl Into<String>) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    }
}

#[derive(Default)]
struct ConversationState {
    prompts: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
struct CodingQuestionProvider {
    state: Arc<ConversationState>,
}

impl CodingQuestionProvider {
    fn prompts(&self) -> Vec<String> {
        self.state.prompts.lock().clone()
    }
}

#[async_trait]
impl Provider for CodingQuestionProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let flattened = request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.state.prompts.lock().push(flattened.clone());

        if flattened.contains("ORCH_ANSWER_USE_RPC") {
            return Ok(text_response(
                "CODE_AGENT_DONE: implemented controller-registry route after orchestrator answer",
            ));
        }

        Ok(text_response(
            "CODE_AGENT_QUESTION: should this use controller registry or direct jsonrpc branch?",
        ))
    }
}

#[derive(Default)]
struct ParallelState {
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
struct ParallelCodingProvider {
    state: Arc<ParallelState>,
}

impl ParallelCodingProvider {
    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn max_active(&self) -> usize {
        self.state.max_active.load(Ordering::SeqCst)
    }

    fn prompts(&self) -> Vec<String> {
        self.state.prompts.lock().clone()
    }

    fn record_peak(&self, current: usize) {
        let mut observed = self.state.max_active.load(Ordering::SeqCst);
        while current > observed {
            match self.state.max_active.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }
}

#[async_trait]
impl Provider for ParallelCodingProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let current = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_peak(current);
        sleep(Duration::from_millis(35)).await;

        let flattened = request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.state.prompts.lock().push(flattened.clone());
        self.state.active.fetch_sub(1, Ordering::SeqCst);

        if flattened.contains("PARALLEL_ALPHA") {
            Ok(text_response("ALPHA_DONE"))
        } else if flattened.contains("PARALLEL_BETA") {
            Ok(text_response("BETA_DONE"))
        } else if flattened.contains("PARALLEL_GAMMA") {
            Ok(text_response("GAMMA_DONE"))
        } else {
            Ok(text_response("UNKNOWN_DONE"))
        }
    }
}

#[test]
fn unit_status_serializes_as_snake_case() {
    let value = serde_json::to_value(AgentStatus::Completed).expect("serialize status");
    assert_eq!(value, serde_json::json!("completed"));
}

#[tokio::test]
async fn unit_message_agent_rejects_empty_parent_reply() {
    let session = AgentOrchestrationSession::new("unit-session");
    let error = session
        .message_agent(MessageAgentRequest {
            orchestration_id: "agent-1".to_string(),
            content: "   ".to_string(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, OrchestrationError::InvalidMessage));
}

#[tokio::test]
async fn e2e_orchestrator_answers_coding_agent_question_and_resumes_child() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let provider = CodingQuestionProvider::default();
    let parent = parent_context(Arc::new(provider.clone()));
    let session = AgentOrchestrationSession::new("orchestrator-session");

    let first = with_parent_context(parent.clone(), async {
        session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "code_executor".to_string(),
                prompt: "Implement RPC wiring for AGENT_ORCH_E2E".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await
    })
    .await
    .expect("spawn coding agent");

    let first_wait = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec![first.orchestration_id.clone()],
            timeout_ms: Some(2_000),
        })
        .await
        .expect("wait first child");
    let first_child = &first_wait.agents[0];
    assert_eq!(first_child.status, AgentStatus::Completed);
    assert!(first_child
        .result_summary
        .as_deref()
        .unwrap_or_default()
        .contains("CODE_AGENT_QUESTION"));

    let answered_snapshot = session
        .message_agent(MessageAgentRequest {
            orchestration_id: first.orchestration_id.clone(),
            content: "ORCH_ANSWER_USE_RPC: use controller registry, not direct jsonrpc branch"
                .to_string(),
        })
        .await
        .expect("orchestrator records answer");
    assert_eq!(answered_snapshot.status, AgentStatus::Completed);
    assert_eq!(answered_snapshot.messages.len(), 1);
    assert!(answered_snapshot.messages[0]
        .content
        .contains("ORCH_ANSWER_USE_RPC"));

    let follow_up = with_parent_context(parent, async {
        session
            .follow_up(FollowUpRequest {
                orchestration_id: first.orchestration_id.clone(),
                prompt: "Continue after the orchestrator answered: ORCH_ANSWER_USE_RPC".to_string(),
                context: Some("Parent answered: use controller registry".to_string()),
            })
            .await
    })
    .await
    .expect("spawn follow-up coding child");

    let final_wait = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec![follow_up.orchestration_id.clone()],
            timeout_ms: Some(2_000),
        })
        .await
        .expect("wait follow-up");
    let final_child = &final_wait.agents[0];
    assert_eq!(
        final_child.parent_agent_id.as_deref(),
        Some(first.orchestration_id.as_str())
    );
    assert_eq!(final_child.status, AgentStatus::Completed);
    assert!(final_child
        .result_summary
        .as_deref()
        .unwrap_or_default()
        .contains("CODE_AGENT_DONE"));

    let prompts = provider.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("AGENT_ORCH_E2E"));
    assert!(prompts[1].contains("ORCH_ANSWER_USE_RPC"));
}

#[tokio::test]
async fn e2e_orchestrator_waits_for_multiple_parallel_coding_subagents() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let provider = ParallelCodingProvider::default();
    let parent = parent_context(Arc::new(provider.clone()));
    let session = AgentOrchestrationSession::new("parallel-orchestrator-session");

    let spawned = with_parent_context(parent, async {
        let alpha = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "code_executor".to_string(),
                prompt: "Work independently on PARALLEL_ALPHA".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await?;
        let beta = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "code_executor".to_string(),
                prompt: "Work independently on PARALLEL_BETA".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await?;
        let gamma = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "code_executor".to_string(),
                prompt: "Work independently on PARALLEL_GAMMA".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await?;
        Ok::<_, OrchestrationError>(vec![
            alpha.orchestration_id,
            beta.orchestration_id,
            gamma.orchestration_id,
        ])
    })
    .await
    .expect("spawn parallel coding agents");

    let waited = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: spawned,
            timeout_ms: Some(2_000),
        })
        .await
        .expect("wait parallel children");

    assert!(waited.completed);
    assert_eq!(waited.agents.len(), 3);
    assert!(waited
        .agents
        .iter()
        .all(|agent| agent.status == AgentStatus::Completed));
    let outputs = waited
        .agents
        .iter()
        .filter_map(|agent| agent.result_summary.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(outputs.contains("ALPHA_DONE"));
    assert!(outputs.contains("BETA_DONE"));
    assert!(outputs.contains("GAMMA_DONE"));
    assert_eq!(provider.calls(), 3);
    assert!(
        provider.max_active() >= 2,
        "expected overlapping subagent calls, max_active={}",
        provider.max_active()
    );
    let prompts = provider.prompts().join("\n");
    assert!(prompts.contains("PARALLEL_ALPHA"));
    assert!(prompts.contains("PARALLEL_BETA"));
    assert!(prompts.contains("PARALLEL_GAMMA"));
}
