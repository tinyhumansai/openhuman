use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use openhuman_core::openhuman::agent::harness::session::Agent;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, ParentExecutionContext, PromptSource,
    SandboxMode, SubagentRunOptions, ToolScope,
};
use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::ToolCallFormat;
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary};
use openhuman_core::openhuman::tools::SpawnSubagentTool;
use openhuman_core::openhuman::tools::{Tool, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct ScriptedProvider {
    responses: Mutex<Vec<ChatResponse>>,
    requests: Mutex<Vec<Vec<ChatMessage>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Vec<ChatMessage>> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok(format!("direct: {message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(request.messages.to_vec());
        let mut responses = self.responses.lock();
        Ok(if responses.is_empty() {
            ChatResponse {
                text: Some("fallback final".to_string()),
                tool_calls: vec![],
                usage: Some(usage(7, 3)),
                reasoning_content: None,
            }
        } else {
            responses.remove(0)
        })
    }
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "stub-memory"
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo a deterministic payload for harness coverage"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let message = args
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("(missing)");
        Ok(ToolResult::success(format!("echoed:{message}")))
    }
}

fn usage(input_tokens: u64, output_tokens: u64) -> UsageInfo {
    usage_with_cached(input_tokens, output_tokens, input_tokens / 2)
}

fn usage_with_cached(input_tokens: u64, output_tokens: u64, cached_input_tokens: u64) -> UsageInfo {
    UsageInfo {
        input_tokens,
        output_tokens,
        context_window: 8_192,
        cached_input_tokens,
        charged_amount_usd: 0.001,
    }
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
        extra_content: None,
    }
}

fn response(
    text: Option<&str>,
    tool_calls: Vec<ToolCall>,
    input: u64,
    output: u64,
) -> ChatResponse {
    ChatResponse {
        text: text.map(str::to_string),
        tool_calls,
        usage: Some(usage(input, output)),
        reasoning_content: None,
    }
}

fn response_with_cached(
    text: Option<&str>,
    tool_calls: Vec<ToolCall>,
    input: u64,
    output: u64,
    cached: u64,
) -> ChatResponse {
    ChatResponse {
        text: text.map(str::to_string),
        tool_calls,
        usage: Some(usage_with_cached(input, output, cached)),
        reasoning_content: None,
    }
}

fn agent_config() -> AgentConfig {
    AgentConfig {
        max_tool_iterations: 4,
        max_history_messages: 12,
        ..AgentConfig::default()
    }
}

fn build_agent(
    workspace: &Path,
    provider: Arc<ScriptedProvider>,
    agent_name: &str,
) -> Result<Agent> {
    build_agent_with_tools(workspace, provider, agent_name, vec![Box::new(EchoTool)])
}

fn build_agent_with_tools(
    workspace: &Path,
    provider: Arc<ScriptedProvider>,
    agent_name: &str,
    tools: Vec<Box<dyn Tool>>,
) -> Result<Agent> {
    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(tools)
        .memory(Arc::new(StubMemory))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .config(agent_config())
        .model_name("coverage-model".to_string())
        .temperature(0.0)
        .workspace_dir(workspace.to_path_buf())
        .workflows(Vec::new())
        .auto_save(false)
        .event_context("coverage-session", "coverage-channel")
        .agent_definition_name(agent_name)
        .omit_profile(true)
        .omit_memory_md(true)
        .build()?;
    agent.set_connected_integrations(Vec::new());
    Ok(agent)
}

fn parent_context(workspace: PathBuf, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let tool_specs = tools.iter().map(|tool| tool.spec()).collect();
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: [
            "test".to_string(),
            "researcher".to_string(),
            "code_executor".to_string(),
        ]
        .into_iter()
        .collect(),
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        model_name: "coverage-model".to_string(),
        temperature: 0.0,
        workspace_dir: workspace,
        memory: Arc::new(StubMemory),
        agent_config: agent_config(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory context".to_string())),
        session_id: "parent-session".to_string(),
        channel: "coverage-channel".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "1700000000_parent".to_string(),
        session_parent_prefix: Some("root-chain".to_string()),
        on_progress: None,
        run_queue: None,
    }
}

fn coverage_definition() -> AgentDefinition {
    AgentDefinition {
        id: "coverage_worker".to_string(),
        when_to_use: "Used by raw integration coverage tests".to_string(),
        display_name: Some("Coverage Worker".to_string()),
        system_prompt: PromptSource::Inline("Answer only from deterministic test tools.".into()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: Default::default(),
        temperature: 0.0,
        tools: ToolScope::Named(vec!["echo".to_string()]),
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 3,
        iteration_policy: Default::default(),
        max_result_chars: Some(18),
        timeout_secs: None,
        sandbox_mode: SandboxMode::ReadOnly,
        background: false,
        trigger_memory_agent: Default::default(),
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: Default::default(),
    }
}

fn transcript_jsonl_files(workspace: &Path) -> Vec<PathBuf> {
    let session_raw = workspace.join("session_raw");
    let mut files = std::fs::read_dir(session_raw)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[tokio::test]
async fn agent_turn_executes_tools_persists_and_resumes_raw_transcript() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let provider = Arc::new(ScriptedProvider::new(vec![
        response(
            Some("calling echo"),
            vec![tool_call("call-1", "echo", json!({"message": "alpha"}))],
            120,
            8,
        ),
        response(Some("final after echo"), Vec::new(), 132, 11),
    ]));
    let mut agent = build_agent(workspace.path(), provider.clone(), "coverage_main")?;

    let first = agent.turn("please call echo").await?;
    assert_eq!(first, "final after echo");
    assert!(agent.history().len() >= 4);

    let files = transcript_jsonl_files(workspace.path());
    assert_eq!(files.len(), 1, "expected one root transcript: {files:?}");
    let transcript = std::fs::read_to_string(&files[0])?;
    assert!(transcript.contains("\"agent\":\"coverage_main\""));
    assert!(transcript.contains("final after echo"));
    assert!(transcript.contains("\"input_tokens\":252"));
    assert!(workspace.path().join("sessions").exists());

    let resume_provider = Arc::new(ScriptedProvider::new(vec![response(
        Some("resumed answer"),
        Vec::new(),
        64,
        6,
    )]));
    let mut resumed = build_agent(workspace.path(), resume_provider.clone(), "coverage_main")?;
    let second = resumed.turn("continue from transcript").await?;
    assert_eq!(second, "resumed answer");

    let requests = resume_provider.requests();
    let first_request = requests.first().expect("provider should be called");
    assert!(
        first_request
            .iter()
            .any(|message| message.role == "assistant" && message.content == "final after echo"),
        "resume request should include assistant message from prior transcript: {first_request:#?}"
    );

    Ok(())
}

#[tokio::test]
async fn run_subagent_filters_tools_runs_inner_loop_and_writes_child_transcript() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let provider = Arc::new(ScriptedProvider::new(vec![
        response(
            Some("need a tool"),
            vec![tool_call("sub-call-1", "echo", json!({"message": "beta"}))],
            90,
            5,
        ),
        response(
            Some("subagent final response that will be capped by definition"),
            Vec::new(),
            101,
            7,
        ),
    ]));
    let parent = parent_context(workspace.path().to_path_buf(), provider.clone());
    let definition = coverage_definition();

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &definition,
            "Use echo once and then summarize.",
            SubagentRunOptions {
                task_id: Some("coverage-task".to_string()),
                context: Some("caller supplied context".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await
    })
    .await?;

    assert_eq!(outcome.agent_id, "coverage_worker");
    assert_eq!(outcome.iterations, 2);
    assert_eq!(outcome.output, "subagent final res\n[...truncated]");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let first_request = requests.first().expect("subagent provider request");
    assert!(
        first_request.iter().any(|message| message.role == "user"
            && message.content.contains("parent memory context")
            && message.content.contains("caller supplied context")),
        "subagent user prompt should merge parent and caller context: {first_request:#?}"
    );

    let files = transcript_jsonl_files(workspace.path());
    assert_eq!(files.len(), 1, "expected one child transcript: {files:?}");
    let stem = files[0]
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    assert!(stem.starts_with("root-chain__1700000000_parent__"));
    assert!(stem.contains("coverage_worker"));

    let transcript = std::fs::read_to_string(&files[0])?;
    assert!(transcript.contains("\"agent\":\"coverage_worker\""));
    assert!(transcript.contains("echoed:beta"));
    assert!(transcript.contains("\"input_tokens\":191"));

    Ok(())
}

#[tokio::test]
async fn repeated_subagent_spawns_keep_cacheable_prefix_and_record_provider_cache_hit() -> Result<()>
{
    let workspace = tempfile::tempdir()?;
    let provider = Arc::new(ScriptedProvider::new(vec![
        response_with_cached(Some("first"), Vec::new(), 100, 5, 0),
        response_with_cached(Some("second"), Vec::new(), 100, 5, 88),
    ]));
    let parent = parent_context(workspace.path().to_path_buf(), provider.clone());
    let definition = coverage_definition();

    let (first, second) = with_parent_context(parent, async {
        let first = run_subagent(
            &definition,
            "Answer the stable cache probe.",
            SubagentRunOptions {
                task_id: Some("cache-probe-a".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await?;
        let second = run_subagent(
            &definition,
            "Answer the stable cache probe.",
            SubagentRunOptions {
                task_id: Some("cache-probe-b".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await?;
        Ok::<_, openhuman_core::openhuman::agent::harness::SubagentRunError>((first, second))
    })
    .await?;

    assert_eq!(first.output, "first");
    assert_eq!(second.output, "second");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let first_system = requests[0]
        .iter()
        .find(|message| message.role == "system")
        .expect("first subagent request should include a system prompt");
    let second_system = requests[1]
        .iter()
        .find(|message| message.role == "system")
        .expect("second subagent request should include a system prompt");
    assert_eq!(
        first_system.content, second_system.content,
        "repeated subagent spawns of the same definition must preserve the byte-identical \
         system prefix the backend can cache"
    );

    let transcripts = transcript_jsonl_files(workspace.path());
    assert_eq!(
        transcripts.len(),
        2,
        "each subagent run should persist its own raw transcript: {transcripts:?}"
    );
    let joined = transcripts
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?
        .join("\n");
    assert!(
        joined.contains("\"cached_input_tokens\":88"),
        "provider-reported cached input tokens from the second child run should be preserved \
         in subagent transcript accounting:\n{joined}"
    );

    Ok(())
}

#[tokio::test]
async fn orchestrator_spawn_subagent_round_trip_streams_child_events_and_returns_result(
) -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let agents_dir = workspace.path().join("agents");
    std::fs::create_dir_all(&agents_dir)?;
    std::fs::write(
        agents_dir.join("coverage_orchestrator.toml"),
        r#"
id = "coverage_orchestrator"
display_name = "Coverage Orchestrator"
when_to_use = "Deterministic parent agent used by harness cache tests."
temperature = 0.0
max_iterations = 3
agent_tier = "chat"
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = true
omit_memory_md = true

[system_prompt]
inline = "Delegate the cache probe and synthesize the result."

[subagents]
allowlist = ["cache_probe_child"]
"#,
    )?;
    std::fs::write(
        agents_dir.join("cache_probe_child.toml"),
        r#"
id = "cache_probe_child"
display_name = "Cache Probe Child"
when_to_use = "Deterministic child agent used by harness cache tests."
temperature = 0.0
max_iterations = 3
omit_identity = true
omit_memory_context = true
omit_safety_preamble = true
omit_skills_catalog = true
omit_profile = true
omit_memory_md = true

[system_prompt]
inline = "Answer the delegated cache probe directly."
"#,
    )?;
    let _ = AgentDefinitionRegistry::init_global(workspace.path());

    let child_answer = "child-cache-observation: prefix was reusable";
    let parent_final = "orchestrator final: child-cache-observation accepted";
    let provider = Arc::new(ScriptedProvider::new(vec![
        response(
            Some("delegating to child"),
            vec![tool_call(
                "spawn-child-1",
                "spawn_subagent",
                json!({
                    "agent_id": "cache_probe_child",
                    "prompt": "Inspect whether the child turn can answer a cache probe.",
                    "context": "Parent observed request id cache-42.",
                }),
            )],
            140,
            9,
        ),
        response_with_cached(Some(child_answer), Vec::new(), 96, 7, 64),
        response(Some(parent_final), Vec::new(), 120, 10),
    ]));
    let mut agent = build_agent_with_tools(
        workspace.path(),
        provider.clone(),
        "coverage_orchestrator",
        vec![Box::new(SpawnSubagentTool::new())],
    )?;
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(1024);
    agent.set_on_progress(Some(progress_tx));

    let answer = agent
        .turn("Ask a child agent for the cache observation, then respond.")
        .await?;
    assert_eq!(
        answer, parent_final,
        "orchestrator should synthesize after receiving the child result"
    );

    let mut progress = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        progress.push(event);
    }
    assert!(
        progress.iter().any(|event| matches!(
            event,
            AgentProgress::SubagentSpawned { agent_id, task_id, .. }
                if agent_id == "cache_probe_child" && task_id.starts_with("sub-")
        )),
        "parent progress should announce the child spawn: {progress:#?}"
    );
    assert!(
        progress.iter().any(|event| matches!(
            event,
            AgentProgress::SubagentCompleted { agent_id, output_chars, .. }
                if agent_id == "cache_probe_child" && *output_chars == child_answer.chars().count()
        )),
        "parent progress should announce child completion: {progress:#?}"
    );

    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "expected parent request, child request, then parent synthesis request"
    );
    assert!(
        requests[0]
            .iter()
            .any(|message| message.role == "user" && message.content.contains("Ask a child agent")),
        "first request should be the orchestrator turn: {:#?}",
        requests[0]
    );
    assert!(
        requests[1].iter().any(|message| message.role == "system"
            && message.content.contains("Sub-agent Role Contract"))
            && requests[1].iter().any(|message| message.role == "user"
                && message
                    .content
                    .contains("Parent observed request id cache-42")),
        "second request should be the child subagent turn with parent-supplied context: {:#?}",
        requests[1]
    );
    assert!(
        requests[2]
            .iter()
            .any(|message| message.role == "tool" && message.content.contains(child_answer)),
        "third request should return the child result to the orchestrator as a tool result: {:#?}",
        requests[2]
    );
    let transcripts = transcript_jsonl_files(workspace.path());
    let joined = transcripts
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?
        .join("\n");
    assert!(
        joined.contains("\"agent\":\"cache_probe_child\"")
            && joined.contains(child_answer)
            && joined.contains("\"cached_input_tokens\":64"),
        "child transcript should be persisted alongside the parent turn:\n{joined}"
    );

    Ok(())
}
