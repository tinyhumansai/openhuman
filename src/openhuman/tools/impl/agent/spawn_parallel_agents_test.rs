use super::*;
use crate::openhuman::agent::dispatcher::NativeToolDispatcher;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::AgentConfig;
use crate::openhuman::context::prompt::ToolCallFormat;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::providers::traits::ProviderCapabilities;
use crate::openhuman::providers::{
    ChatRequest, ChatResponse, ConversationMessage, Provider, ToolCall,
};
use crate::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::time::{sleep, Duration};

const PARENT_PROMPT_CANARY: &str = "parallel-fanout-e2e-canary";
const RESEARCH_PROMPT_CANARY: &str = "research-branch-canary";
const PLANNER_PROMPT_CANARY: &str = "planner-branch-canary";
const RESEARCH_DONE_CANARY: &str = "research-finished-canary";
const PLANNER_DONE_CANARY: &str = "planner-finished-canary";
const FINAL_CANARY: &str = "parallel-summary-canary";

#[test]
fn metadata_methods_expose_execute_permission_and_schema() {
    let tool = SpawnParallelAgentsTool::default();
    assert_eq!(tool.name(), "spawn_parallel_agents");
    assert!(tool.description().contains("independent sub-agent tasks"));
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"][0], "tasks");
    assert_eq!(schema["properties"]["tasks"]["minItems"], 2);
}

#[test]
fn ownership_boundary_is_prepended_when_present() {
    let prompt = with_ownership_boundary("implement tests", Some("files: src/foo.rs"));
    assert!(prompt.starts_with("[Ownership Boundary]"));
    assert!(prompt.contains("files: src/foo.rs"));
    assert!(prompt.contains("[Task]\nimplement tests"));
}

#[tokio::test]
async fn rejects_single_task() {
    let tool = SpawnParallelAgentsTool::new();
    let result = tool
        .execute(json!({
            "tasks": [{ "agent_id": "researcher", "prompt": "only one" }]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("at least two"));
}

#[tokio::test]
async fn rejects_missing_or_invalid_tasks_before_parent_lookup() {
    let tool = SpawnParallelAgentsTool::new();

    let missing = tool.execute(json!({})).await.expect_err("missing tasks");
    assert!(missing.to_string().contains("Missing 'tasks'"));

    let invalid = tool
        .execute(json!({ "tasks": "not an array" }))
        .await
        .expect_err("invalid tasks");
    assert!(invalid.to_string().contains("Invalid tasks array"));
}

#[tokio::test]
async fn rejects_two_tasks_outside_agent_turn() {
    let tool = SpawnParallelAgentsTool::new();
    let result = tool
        .execute(json!({
            "tasks": [
                { "agent_id": "researcher", "prompt": "one" },
                { "agent_id": "planner", "prompt": "two" }
            ]
        }))
        .await
        .expect("tool result");
    assert!(result.is_error);
    assert!(result.output().contains("outside of an agent turn"));
}

struct NoopProvider;

#[async_trait]
impl Provider for NoopProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: Vec::new(),
            usage: None,
        })
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

fn parent_context_with_provider(
    max_parallel_tools: usize,
    provider: Arc<dyn Provider>,
) -> ParentExecutionContext {
    let agent_config = AgentConfig {
        max_parallel_tools,
        ..Default::default()
    };
    ParentExecutionContext {
        provider,
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: std::env::temp_dir(),
        memory: Arc::new(NoopMemory),
        agent_config,
        skills: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "session-test".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
    }
}

fn parent_context(max_parallel_tools: usize) -> ParentExecutionContext {
    parent_context_with_provider(max_parallel_tools, Arc::new(NoopProvider))
}

#[tokio::test]
async fn rejects_more_tasks_than_parent_parallel_limit() {
    let tool = SpawnParallelAgentsTool::new();
    let parent = parent_context(2);
    let result = with_parent_context(parent, async {
        tool.execute(json!({
            "tasks": [
                { "agent_id": "researcher", "prompt": "one" },
                { "agent_id": "planner", "prompt": "two" },
                { "agent_id": "critic", "prompt": "three" }
            ]
        }))
        .await
    })
    .await
    .expect("tool result");
    assert!(result.is_error);
    assert!(result.output().contains("max_parallel_tools"));
}

#[tokio::test]
async fn collects_immediate_task_validation_failures() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = SpawnParallelAgentsTool::new();
    let parent = parent_context(4);

    let result = with_parent_context(parent, async {
        tool.execute(json!({
            "tasks": [
                { "agent_id": " ", "prompt": "missing agent", "ownership": "files: none" },
                { "agent_id": "__missing_agent__", "prompt": "unknown agent" },
                { "agent_id": "integrations_agent", "prompt": "needs toolkit" }
            ]
        }))
        .await
    })
    .await
    .expect("tool result");

    assert!(!result.is_error, "{}", result.output());
    let body: serde_json::Value = serde_json::from_str(&result.output()).expect("json output");
    assert_eq!(body["parallel_agents"]["total"], 3);
    assert_eq!(body["parallel_agents"]["failed"], 3);
    let errors = body["parallel_agents"]["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["error"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(errors
        .iter()
        .any(|error| error.contains("agent_id and prompt")));
    assert!(errors
        .iter()
        .any(|error| error.contains("unknown agent_id")));
    assert!(errors
        .iter()
        .any(|error| error.contains("requires toolkit")));
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

#[derive(Default)]
struct ParallelHarnessState {
    total_calls: AtomicUsize,
    active_subagent_calls: AtomicUsize,
    max_active_subagent_calls: AtomicUsize,
    seen_payloads: Mutex<Vec<String>>,
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

    async fn respond_for_subagent(&self, flattened: &str) -> anyhow::Result<ChatResponse> {
        let current = self
            .state
            .active_subagent_calls
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        self.record_active_peak(current);
        sleep(Duration::from_millis(25)).await;

        let response = (|| -> anyhow::Result<ChatResponse> {
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
                anyhow::bail!("unexpected subagent payload: {flattened}");
            }
        })();

        self.state
            .active_subagent_calls
            .fetch_sub(1, Ordering::SeqCst);
        response
    }
}

#[async_trait]
impl Provider for ParallelHarnessProvider {
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
        Ok("ok".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.state.total_calls.fetch_add(1, Ordering::SeqCst);
        let flattened = request
            .messages
            .iter()
            .map(|m| format!("{}:{}", m.role, m.content))
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

fn text_response(text: impl Into<String>) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: Vec::new(),
        usage: None,
    }
}

fn tool_response(name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments: arguments.to_string(),
        }],
        usage: None,
    }
}

#[tokio::test]
async fn agent_turn_runs_long_parallel_subagent_flow_with_many_nested_tool_calls() {
    AgentDefinitionRegistry::init_global_builtins().unwrap();

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    let provider = ParallelHarnessProvider::default();
    let fixture_state = Arc::new(FixtureStepState::default());

    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path).unwrap());

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(SpawnParallelAgentsTool::new()),
        Box::new(FixtureStepTool {
            state: Arc::clone(&fixture_state),
        }),
    ];

    let mut agent = Agent::builder()
        .provider(Box::new(provider.clone()))
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
