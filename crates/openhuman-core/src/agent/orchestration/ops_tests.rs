use super::*;
use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::agent::prompts::ToolCallFormat;
use crate::config::AgentConfig;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::future::Future;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinytools::Tool;
use tokio::time::Duration;

fn parent_context(model: Arc<dyn ChatModel<()>>) -> ParentExecutionContext {
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".to_string(),
        allowed_subagent_ids: ["task_manager_agent".to_string()].into_iter().collect(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model_with_profile(
            model,
            ModelProfile {
                tool_calling: true,
                parallel_tool_calls: true,
                ..ModelProfile::default()
            },
        ),
        all_tools: Arc::new(Vec::<Box<dyn Tool>>::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".to_string(),
        temperature: 0.2,
        workspace_dir: std::env::temp_dir(),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "orchestrator-session".to_string(),
        channel: "test".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_orchestrator".to_string(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn text_response(text: impl Into<String>) -> ModelResponse {
    ModelResponse::assistant(text)
}

#[derive(Default)]
struct ConversationState {
    prompts: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
struct CodingQuestionModel {
    state: Arc<ConversationState>,
}

impl CodingQuestionModel {
    fn prompts(&self) -> Vec<String> {
        self.state.prompts.lock().clone()
    }
}

#[async_trait]
impl ChatModel<()> for CodingQuestionModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let flattened = request
            .messages
            .iter()
            .map(|message| message.text())
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

/// Number of parallel sub-agents the parallel-coding test spawns. The model's
/// synchronization barrier is sized to this so the peak-concurrency assertion is
/// deterministic regardless of scheduler/load.
const PARALLEL_CHILDREN: usize = 3;

struct ParallelState {
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    /// Rendezvous point: every child parks here (yielding its worker thread)
    /// until all `PARALLEL_CHILDREN` are concurrently inside `chat`, so
    /// `max_active` deterministically reaches the peak instead of depending on
    /// whether the brief model calls happen to overlap in wall-clock time.
    gate: tokio::sync::Barrier,
}

impl Default for ParallelState {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            gate: tokio::sync::Barrier::new(PARALLEL_CHILDREN),
        }
    }
}

#[derive(Clone, Default)]
struct ParallelCodingModel {
    state: Arc<ParallelState>,
}

impl ParallelCodingModel {
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
impl ChatModel<()> for ParallelCodingModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        let current = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_peak(current);
        // Park until all children have entered `chat` (or a generous timeout, so
        // an unexpected missing child fails the assertion fast rather than
        // hanging). Once released, every child was concurrently active, so the
        // recorded peak equals `PARALLEL_CHILDREN`.
        let _ = tokio::time::timeout(Duration::from_secs(10), self.state.gate.wait()).await;

        let flattened = request
            .messages
            .iter()
            .map(|message| message.text())
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
    let value = serde_json::to_value(OrchestrationTaskStatus::Completed).expect("serialize status");
    assert_eq!(value, serde_json::json!("completed"));
}

#[tokio::test]
async fn e2e_orchestrator_answers_coding_agent_question_and_resumes_child() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let provider = CodingQuestionModel::default();
    let parent = parent_context(Arc::new(provider.clone()));
    let session = AgentOrchestrationSession::new("orchestrator-session");

    let first = with_parent_context(parent.clone(), async {
        session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
                prompt: "Implement RPC wiring for AGENT_ORCH_E2E".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await
    })
    .await
    .expect("spawn coding agent");

    // These waits spawn a *real* builtin (`task_manager_agent`) sub-agent on the
    // detached executor, which builds the full agent (prompt assembly, tool
    // resolution, registry) before the mock model returns — ~2.7s per child.
    // The wait budget must clear that with CI headroom; a tight 2s expires first
    // and reports the child as `Running`.
    let first_wait = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec![first.orchestration_id.clone()],
            timeout_ms: Some(15_000),
        })
        .await
        .expect("wait first child");
    let first_child = &first_wait.agents[0];
    assert_eq!(first_child.status, OrchestrationTaskStatus::Completed);
    assert!(first_child
        .result_summary
        .as_deref()
        .unwrap_or_default()
        .contains("CODE_AGENT_QUESTION"));

    // `message_agent`/`follow_up` are gone (superseded by the durable verbs in
    // `command_center::control`). The orchestrator now records its answer by
    // spawning the linked continuation child directly — the same thing
    // `follow_up` did internally.
    let follow_up = with_parent_context(parent, async {
        session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
                prompt: "Continue after the orchestrator answered: ORCH_ANSWER_USE_RPC".to_string(),
                context: Some("Parent answered: use controller registry".to_string()),
                model: Some("test-model".to_string()),
                parent_agent_id: Some(first.orchestration_id.clone()),
                ..Default::default()
            })
            .await
    })
    .await
    .expect("spawn follow-up coding child");

    let final_wait = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec![follow_up.orchestration_id.clone()],
            timeout_ms: Some(15_000),
        })
        .await
        .expect("wait follow-up");
    let final_child = &final_wait.agents[0];
    assert_eq!(
        final_child.parent_agent_id.as_deref(),
        Some(first.orchestration_id.as_str())
    );
    assert_eq!(final_child.status, OrchestrationTaskStatus::Completed);
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

// Multi-thread runtime: this test asserts the three detached sub-agents run
// *concurrently* (`max_active >= 2`). Each child does a CPU-bound builtin-agent
// build before its (mock) model call; on a single-threaded runtime those
// builds serialize, so the brief model calls never overlap and the peak-
// concurrency assertion flakes under load. Real worker threads let the builds —
// and therefore the model calls — actually overlap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_orchestrator_waits_for_multiple_parallel_coding_subagents() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let provider = ParallelCodingModel::default();
    let parent = parent_context(Arc::new(provider.clone()));
    let session = AgentOrchestrationSession::new("parallel-orchestrator-session");

    let spawned = with_parent_context(parent, async {
        let alpha = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
                prompt: "Work independently on PARALLEL_ALPHA".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await?;
        let beta = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
                prompt: "Work independently on PARALLEL_BETA".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await?;
        let gamma = session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
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
            timeout_ms: Some(15_000),
        })
        .await
        .expect("wait parallel children");

    assert!(waited.completed);
    assert_eq!(waited.agents.len(), 3);
    assert!(waited
        .agents
        .iter()
        .all(|agent| agent.status == OrchestrationTaskStatus::Completed));
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

/// A model that parks long enough for `abort_all` to land while the child is
/// still running.
#[derive(Clone, Default)]
struct BlockingModel;

#[async_trait]
impl ChatModel<()> for BlockingModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(text_response("NEVER_REACHED"))
    }
}

#[tokio::test]
async fn unit_wait_agents_with_no_ids_returns_an_empty_complete_response() {
    let session = AgentOrchestrationSession::new("empty-session");
    let response = session
        .wait_agents(WaitAgentOptions::default())
        .await
        .expect("empty wait succeeds");

    assert!(response.completed);
    assert!(response.agents.is_empty());
}

#[tokio::test]
async fn unit_wait_agents_rejects_an_unknown_child() {
    let session = AgentOrchestrationSession::new("unknown-session");
    let error = session
        .wait_agents(WaitAgentOptions {
            orchestration_ids: vec!["agent-does-not-exist".to_string()],
            timeout_ms: Some(50),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(&error, OrchestrationError::AgentNotFound(id) if id == "agent-does-not-exist"),
        "unexpected error: {error:?}"
    );
}

/// `abort_all` must publish a terminal `Cancelled` status on each live child's
/// watch channel *before* the crate registry hard-aborts it, so a waiter in
/// flight resolves with a cancellation rather than a closed status channel.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_abort_all_cancels_an_in_flight_child_for_a_concurrent_waiter() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let parent = parent_context(Arc::new(BlockingModel));
    let session = AgentOrchestrationSession::new("abort-session");

    let spawned = with_parent_context(parent, async {
        session
            .spawn_agent(SpawnAgentRequest {
                agent_id: "task_manager_agent".to_string(),
                prompt: "Park until the orchestrator interrupts".to_string(),
                model: Some("test-model".to_string()),
                ..Default::default()
            })
            .await
    })
    .await
    .expect("spawn blocking child");

    let waiter_session = session.clone();
    let waiter_id = spawned.orchestration_id.clone();
    let mut waiter = Box::pin(async move {
        waiter_session
            .wait_agents(WaitAgentOptions {
                orchestration_ids: vec![waiter_id],
                timeout_ms: Some(30_000),
            })
            .await
    });

    // Observe Running before cancellation, then poll the waiter future once
    // until it suspends inside wait_agents. This ensures its registry snapshot
    // and watch subscription are established before abort_all removes the
    // entry; a task-level start signal alone leaves that lookup racy.
    let observed_running = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = session
                .wait_agents(WaitAgentOptions {
                    orchestration_ids: vec![spawned.orchestration_id.clone()],
                    timeout_ms: Some(5),
                })
                .await
                .expect("poll for readiness succeeds while the child is still live");
            if response.agents[0].status == OrchestrationTaskStatus::Running {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        observed_running.is_ok(),
        "child never reached Running before the readiness timeout"
    );
    futures::future::poll_fn(|cx| match waiter.as_mut().poll(cx) {
        std::task::Poll::Pending => std::task::Poll::Ready(()),
        std::task::Poll::Ready(_) => panic!("waiter completed before cancellation"),
    })
    .await;
    session.abort_all().await;

    let response = waiter.await.expect("wait resolves after abort_all");
    assert!(response.completed);
    assert_eq!(response.agents.len(), 1);
    assert_eq!(
        response.agents[0].status,
        OrchestrationTaskStatus::Cancelled
    );
}
