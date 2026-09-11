use super::*;
use crate::openhuman::agent::context::prompt::ToolCallFormat;
use crate::openhuman::agent::dispatcher::NativeToolDispatcher;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentTier, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
};
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::agent::messages::ConversationMessage;
use crate::openhuman::agent::orchestration::spawn_parallel_graph::{
    prepare_spawn_parallel_tasks_from_defs, ParallelTaskRejectionKind, SpawnParallelTaskPreflight,
    WorkerDispatchMode,
};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::AgentConfig;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::tools::traits::ToolTimeout;
use crate::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinyinference::message::{AssistantMessage, Message};
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyinference::tool::ToolCall;
use tokio::time::{sleep, timeout, Duration};

const PARENT_PROMPT_CANARY: &str = "parallel-fanout-e2e-canary";
const RESEARCH_PROMPT_CANARY: &str = "research-branch-canary";
const PLANNER_PROMPT_CANARY: &str = "planner-branch-canary";
const RESEARCH_DONE_CANARY: &str = "research-finished-canary";
const PLANNER_DONE_CANARY: &str = "planner-finished-canary";
const FINAL_CANARY: &str = "parallel-summary-canary";

fn test_lineage(task_id: &str) -> ParallelAgentLineage {
    ParallelAgentLineage {
        parent_session: "parent-session".into(),
        root_session: "root-session".into(),
        child_task_id: task_id.into(),
    }
}

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

fn parent_context(max_parallel_tools: usize) -> ParentExecutionContext {
    let agent_config = AgentConfig {
        max_parallel_tools,
        ..Default::default()
    };
    let model: Arc<dyn tinyinference::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: [
            "researcher".to_string(),
            "critic".to_string(),
            "integrations_agent".to_string(),
        ]
        .into_iter()
        .collect(),
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(NoopMemory),
        agent_config,
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "session-test".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn parent_context_with_tools(
    max_parallel_tools: usize,
    tools: Vec<Box<dyn Tool>>,
) -> ParentExecutionContext {
    let mut parent = parent_context(max_parallel_tools);
    parent.all_tools = Arc::new(tools);
    parent
}

fn definition_with_tool_scope(
    id: &str,
    tools: ToolScope,
    sandbox_mode: SandboxMode,
) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        when_to_use: "test definition".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("test prompt".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools,
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 3,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression: Default::default(),
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

#[derive(Default)]
struct FixtureStepState {
    calls: AtomicUsize,
}

struct FixtureStepTool {
    state: Arc<FixtureStepState>,
}

#[async_trait]
impl Tool for FixtureStepTool {
    fn name(&self) -> &str {
        "fixture_step"
    }

    fn description(&self) -> &str {
        "Fixture tool used by parallel subagent tests."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["branch", "step"],
            "properties": {
                "branch": { "type": "string" },
                "step": { "type": "integer" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let branch = args
            .get("branch")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let step = args.get("step").and_then(|v| v.as_u64()).unwrap_or(0);
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success(format!("{branch}-step-{step}-ok")))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
}

struct PermissionFixtureTool {
    name: &'static str,
    level: PermissionLevel,
}

#[async_trait]
impl Tool for PermissionFixtureTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Fixture tool with a configurable permission level."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
}

struct ParallelHarnessState {
    total_calls: AtomicUsize,
    active_subagent_calls: AtomicUsize,
    max_active_subagent_calls: AtomicUsize,
    /// Sequence counter over subagent provider calls. The first two calls (one
    /// from each parallel subagent) rendezvous at [`Self::overlap_barrier`].
    subagent_call_seq: AtomicUsize,
    /// Deterministic overlap gate: the first provider call of each parallel
    /// subagent waits here until both have arrived, guaranteeing the
    /// `max_active_subagent_calls >= 2` assertion without depending on a timing
    /// window (the old fixed `sleep` raced under load and flaked — see #5209).
    overlap_barrier: tokio::sync::Barrier,
    seen_payloads: Mutex<Vec<String>>,
}

impl Default for ParallelHarnessState {
    fn default() -> Self {
        Self {
            total_calls: AtomicUsize::new(0),
            active_subagent_calls: AtomicUsize::new(0),
            max_active_subagent_calls: AtomicUsize::new(0),
            subagent_call_seq: AtomicUsize::new(0),
            overlap_barrier: tokio::sync::Barrier::new(2),
            seen_payloads: Mutex::new(Vec::new()),
        }
    }
}

#[derive(Clone, Default)]
struct ParallelHarnessProvider {
    state: Arc<ParallelHarnessState>,
}

impl ParallelHarnessProvider {
    fn total_calls(&self) -> usize {
        self.state.total_calls.load(Ordering::SeqCst)
    }

    fn max_active_subagent_calls(&self) -> usize {
        self.state.max_active_subagent_calls.load(Ordering::SeqCst)
    }

    fn record_active_peak(&self, current: usize) {
        let mut observed = self.state.max_active_subagent_calls.load(Ordering::SeqCst);
        while current > observed {
            match self.state.max_active_subagent_calls.compare_exchange(
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

    async fn respond_for_subagent(&self, flattened: &str) -> tinyinference::Result<ModelResponse> {
        let current = self
            .state
            .active_subagent_calls
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        self.record_active_peak(current);

        // The two parallel subagents' first provider calls rendezvous at the
        // barrier, deterministically forcing them to be active simultaneously
        // (peak >= 2). The old approach slept a fixed 25ms and hoped the second
        // call started before the first woke — a window scheduling jitter could
        // miss, so the `max_active >= 2` assertion flaked run-to-run (#5209).
        // The wait is timeout-guarded so a genuine loss of parallelism fails the
        // assertion instead of hanging the test. Later calls (seq >= 2) yield
        // briefly to keep the interleaving realistic.
        let seq = self.state.subagent_call_seq.fetch_add(1, Ordering::SeqCst);
        if seq < 2 {
            let _ = timeout(Duration::from_secs(5), self.state.overlap_barrier.wait()).await;
        } else {
            sleep(Duration::from_millis(5)).await;
        }

        let response = (|| -> tinyinference::Result<ModelResponse> {
            if flattened.contains(RESEARCH_PROMPT_CANARY) {
                if flattened.contains("research-step-3-ok") {
                    Ok(text_response(RESEARCH_DONE_CANARY))
                } else if flattened.contains("research-step-2-ok") {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "research", "step": 3 }),
                    ))
                } else if flattened.contains("research-step-1-ok") {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "research", "step": 2 }),
                    ))
                } else {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "research", "step": 1 }),
                    ))
                }
            } else if flattened.contains(PLANNER_PROMPT_CANARY) {
                if flattened.contains("planner-step-3-ok") {
                    Ok(text_response(PLANNER_DONE_CANARY))
                } else if flattened.contains("planner-step-2-ok") {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "planner", "step": 3 }),
                    ))
                } else if flattened.contains("planner-step-1-ok") {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "planner", "step": 2 }),
                    ))
                } else {
                    Ok(tool_response(
                        "fixture_step",
                        json!({ "branch": "planner", "step": 1 }),
                    ))
                }
            } else {
                Err(tinyinference::Error::Model(format!(
                    "unexpected subagent payload: {flattened}"
                )))
            }
        })();

        self.state
            .active_subagent_calls
            .fetch_sub(1, Ordering::SeqCst);
        response
    }
}

#[async_trait]
impl ChatModel<()> for ParallelHarnessProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("parallel-harness-test".to_string()),
                tool_calling: true,
                parallel_tool_calls: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.state.total_calls.fetch_add(1, Ordering::SeqCst);
        let flattened = request
            .messages
            .iter()
            .map(|message| {
                let role = match message {
                    Message::System(_) => "system",
                    Message::User(_) => "user",
                    Message::Assistant(_) => "assistant",
                    Message::Tool(_) => "tool",
                };
                format!("{role}:{}", message.text())
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.state.seen_payloads.lock().push(flattened.clone());

        if flattened.contains(PARENT_PROMPT_CANARY) {
            if flattened.contains(RESEARCH_DONE_CANARY) && flattened.contains(PLANNER_DONE_CANARY) {
                return Ok(text_response(format!(
                    "{FINAL_CANARY}: merged {RESEARCH_DONE_CANARY} and {PLANNER_DONE_CANARY}"
                )));
            }

            return Ok(tool_response(
                "spawn_parallel_agents",
                json!({
                    "tasks": [
                        {
                            "agent_id": "__test_inherit_parallel_worker",
                            "prompt": format!("Work the research branch: {RESEARCH_PROMPT_CANARY}"),
                            "ownership": "scope: research"
                        },
                        {
                            "agent_id": "__test_inherit_parallel_worker",
                            "prompt": format!("Work the planning branch: {PLANNER_PROMPT_CANARY}"),
                            "ownership": "scope: planning"
                        }
                    ]
                }),
            ));
        }

        self.respond_for_subagent(&flattened).await
    }
}

fn text_response(text: impl Into<String>) -> ModelResponse {
    ModelResponse::assistant(text)
}

fn tool_response(name: &str, arguments: serde_json::Value) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(format!("call-{name}"), name, arguments)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    }
}

async fn agent_turn_runs_long_parallel_subagent_flow_with_many_nested_tool_calls_inner() {
    // `create_memory` below needs the global embedding host. This test never
    // installed it, so it only passed when some other test file in the same
    // binary happened to run first — and failed outright under any filter
    // narrow enough to exclude them all. `install_for_tests` is `Once`-guarded,
    // so calling it here is free when a sibling already did.
    AgentDefinitionRegistry::init_global_builtins().unwrap();

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    let provider = ParallelHarnessProvider::default();
    let fixture_state = Arc::new(FixtureStepState::default());

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(SpawnParallelAgentsTool::new()),
        Box::new(FixtureStepTool {
            state: Arc::clone(&fixture_state),
        }),
    ];

    let mut agent = Agent::builder()
        .chat_model(Arc::new(provider.clone()))
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent
        .turn("Run a long parallel delegation pass. parallel-fanout-e2e-canary")
        .await
        .unwrap_or_else(|err| {
            panic!(
                "agent turn failed: {err}\nseen payloads:\n{}",
                provider.state.seen_payloads.lock().join("\n---\n")
            )
        });

    assert!(
        response.contains(FINAL_CANARY),
        "final orchestrator response should contain the synthesis canary: {response}"
    );
    assert!(
        response.contains(RESEARCH_DONE_CANARY) && response.contains(PLANNER_DONE_CANARY),
        "final response should include both subagent completions: {response}"
    );
    assert_eq!(
        fixture_state.calls.load(Ordering::SeqCst),
        6,
        "expected three nested tool calls per parallel subagent"
    );
    assert!(
        provider.max_active_subagent_calls() >= 2,
        "expected overlapping subagent provider calls, max_active={}",
        provider.max_active_subagent_calls()
    );
    assert!(
        provider.total_calls() >= 10,
        "expected parent + subagent loop to hit the provider many times, total_calls={}",
        provider.total_calls()
    );

    let history = agent.history();
    let mut saw_parallel_call = false;
    let mut saw_parallel_result = false;
    let mut iterations = Vec::new();

    for message in history {
        match message {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
                if tool_calls
                    .iter()
                    .any(|call| call.name == "spawn_parallel_agents")
                {
                    saw_parallel_call = true;
                }
            }
            ConversationMessage::ToolResults(results) => {
                for result in results {
                    if !result.content.contains("\"parallel_agents\"") {
                        continue;
                    }
                    saw_parallel_result = true;
                    let payload: serde_json::Value =
                        serde_json::from_str(&result.content).expect("parallel tool result json");
                    assert_eq!(payload["parallel_agents"]["succeeded"], 2);
                    assert_eq!(payload["parallel_agents"]["failed"], 0);

                    let results = payload["parallel_agents"]["results"]
                        .as_array()
                        .expect("parallel results array");
                    assert_eq!(results.len(), 2);
                    for item in results {
                        assert_eq!(item["success"], true);
                        iterations.push(
                            item["iterations"]
                                .as_u64()
                                .expect("parallel result iterations"),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    assert!(
        saw_parallel_call,
        "parent history should record spawn_parallel_agents"
    );
    assert!(
        saw_parallel_result,
        "parent history should record the parallel tool result"
    );
    assert_eq!(
        iterations,
        vec![4, 4],
        "each subagent should run three tool calls plus a final completion iteration"
    );
}

/// Helper: parent context whose subagent allowlist admits `ids`.
fn parent_admitting(ids: &[&str], tools: Vec<Box<dyn Tool>>) -> ParentExecutionContext {
    let mut parent = parent_context_with_tools(8, tools);
    parent.allowed_subagent_ids = ids.iter().map(|id| id.to_string()).collect();
    parent
}

/// Helper: the single write-capable tool the dispatch fixtures share.
fn write_fixture_tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(PermissionFixtureTool {
        name: "write_fixture",
        level: PermissionLevel::Write,
    })]
}

/// Helper: build a fan-out task with only the fields a dispatch decision reads.
fn dispatch_task(
    agent_id: &str,
    ownership: Option<&str>,
    isolation: Option<&str>,
) -> ParallelAgentTask {
    ParallelAgentTask {
        agent_id: agent_id.into(),
        prompt: "do the thing".into(),
        context: None,
        toolkit: None,
        ownership: ownership.map(str::to_string),
        isolation: isolation.map(str::to_string),
        base_ref: None,
    }
}

#[path = "spawn_parallel_agents_tests_part_01_tests.rs"]
mod part_01_tests;
