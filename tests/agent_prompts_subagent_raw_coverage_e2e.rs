use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::harness::session::Agent;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, DefinitionSource, ModelSpec,
    ParentExecutionContext, PromptSource, SandboxMode, SubagentRunError, SubagentRunOptions,
    ToolScope,
};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::{
    render_ambient_environment, render_subagent_system_prompt, render_tools, render_user_files,
    ConnectedIntegration, CuratedMemoryPromptSnapshot, LearnedContextData, NamespaceSummary,
    PromptContext, PromptTool, SubagentRenderOptions, SystemPromptBuilder, ToolCallFormat,
    UserIdentity,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary as MemoryNamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<String>>,
    native_tools: bool,
    delay: Option<Duration>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
            native_tools: true,
            delay: None,
        })
    }

    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from([Err(anyhow::anyhow!(message.to_string()))])),
            requests: Mutex::new(Vec::new()),
            native_tools: true,
            delay: None,
        })
    }

    fn delayed(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from([Ok(text_response("late"))])),
            requests: Mutex::new(Vec::new()),
            native_tools: true,
            delay: Some(delay),
        })
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: self.native_tools,
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
        Ok(format!("summary: {message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(
            request
                .messages
                .iter()
                .map(|message| format!("{}:{}", message.role, message.content))
                .collect::<Vec<_>>()
                .join("\n---\n"),
        );
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("fallback final")))
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
        _opts: RecallOpts<'_>,
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

    async fn namespace_summaries(&self) -> Result<Vec<MemoryNamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "round18-memory"
    }
}

struct EchoTool {
    name: &'static str,
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Echoes a deterministic payload"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" },
                "zeta": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success(format!("tool-output:{args}")))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 10,
            output_tokens: 4,
            context_window: 8192,
            cached_input_tokens: 2,
            charged_amount_usd: 0.001,
        }),
        reasoning_content: None,
    }
}

fn tool_response(name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some("calling tool".to_string()),
        tool_calls: vec![ToolCall {
            id: "round18-call".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: Some("test reasoning".to_string()),
    }
}

fn tool(name: &'static str) -> Box<dyn Tool> {
    Box::new(EchoTool { name })
}

fn definition(prompt: PromptSource) -> AgentDefinition {
    AgentDefinition {
        id: "round18_worker".to_string(),
        when_to_use: "raw coverage worker".to_string(),
        display_name: Some("Round 18 Worker".to_string()),
        system_prompt: prompt,
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: false,
        omit_skills_catalog: true,
        omit_profile: false,
        omit_memory_md: false,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec!["echo".to_string()]),
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 3,
        iteration_policy: Default::default(),
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: DefinitionSource::Builtin,
    }
}

fn parent(workspace: PathBuf, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
    let tools = vec![tool("echo"), tool("delegate_nested"), tool("other__skip")];
    let specs = tools.iter().map(|tool| tool.spec()).collect();
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
        all_tool_specs: Arc::new(specs),
        model_name: "round18-model".to_string(),
        temperature: 0.0,
        workspace_dir: workspace,
        memory: Arc::new(StubMemory),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory survives when allowed".to_string())),
        session_id: "round18-session".to_string(),
        channel: "round18".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "1700000000_round18_parent".to_string(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

fn prompt_context<'a>(
    workspace: &'a Path,
    tools: &'a [PromptTool<'a>],
    visible: &'a HashSet<String>,
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: workspace,
        model_name: "round18-model",
        agent_id: "round18_agent",
        tools,
        workflows: &[],
        dispatcher_instructions: "dispatcher rules",
        learned: LearnedContextData {
            reflections: vec!["Prefer direct answers.".to_string(), "   ".to_string()],
            tree_root_summaries: vec![NamespaceSummary {
                namespace: "work".to_string(),
                body: "Long lived work memory.".to_string(),
                updated_at: chrono::DateTime::parse_from_rfc3339("2026-05-29T12:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            }],
            ..LearnedContextData::default()
        },
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[] as &[ConnectedIntegration],
        connected_identities_md: String::new(),
        include_profile: true,
        include_memory_md: true,
        curated_snapshot: Some(Arc::new(CuratedMemoryPromptSnapshot {
            memory: "curated memory body".to_string(),
            user: "curated user body".to_string(),
        })),
        user_identity: Some(UserIdentity {
            id: Some("user-1".to_string()),
            name: Some("Ada\nLovelace".to_string()),
            email: Some("ada@example.test".to_string()),
        }),
        personality_soul_md: Some("personality soul override".to_string()),
        personality_memory_md: None,
        personality_roster: vec![],
    }
}

#[test]
fn prompt_sections_render_files_identity_memory_tools_and_ambient_blocks() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(
        workspace.path().join("PROFILE.md"),
        "profile should be ignored by snapshot",
    )?;
    std::fs::write(
        workspace.path().join("MEMORY.md"),
        "workspace memory fallback",
    )?;
    let prompt_tools = [PromptTool::with_schema(
        "echo",
        "Echo tool",
        json!({"type":"object","properties":{"b":{},"a":{}}}).to_string(),
    )];
    let visible = HashSet::from(["echo".to_string()]);
    let ctx = prompt_context(workspace.path(), &prompt_tools, &visible);

    let rendered = SystemPromptBuilder::with_defaults()
        .insert_section_before(
            "user_memory",
            Box::new(openhuman_core::openhuman::context::prompt::UserReflectionsSection),
        )
        .build(&ctx)?;

    assert!(rendered.contains("personality soul override"));
    assert!(rendered.contains("## User Reflections"));
    assert!(rendered.contains("Prefer direct answers."));
    assert!(rendered.contains("### MEMORY.md"));
    assert!(rendered.contains("curated memory body"));
    assert!(rendered.contains("### USER.md"));
    assert!(rendered.contains("curated user body"));
    assert!(rendered.contains("### work (last updated 2026-05-29)"));
    assert!(rendered.contains("Call as: `echo[a|b]`"));
    assert!(rendered.contains("## Output style"));

    let user_files = render_user_files(&ctx)?;
    assert!(user_files.contains("curated memory body"));
    assert!(!user_files.contains("workspace memory fallback"));

    let ambient = render_ambient_environment(&ctx)?;
    assert!(ambient.contains("name: Ada Lovelace"));
    assert!(ambient.contains("email: ada@example.test"));
    assert!(ambient.contains("## Current Date & Time"));

    let native_ctx = PromptContext {
        tool_call_format: ToolCallFormat::Native,
        dispatcher_instructions: "",
        ..prompt_context(workspace.path(), &prompt_tools, &visible)
    };
    assert_eq!(render_tools(&native_ctx)?, "");

    Ok(())
}

#[test]
fn subagent_prompt_renderer_covers_format_branches_and_missing_indices() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("PROFILE.md"), "profile file").unwrap();
    std::fs::write(workspace.path().join("MEMORY.md"), "memory file").unwrap();
    let parent_tools = vec![tool("alpha")];
    let extra_tools = vec![tool("extra")];
    let options = SubagentRenderOptions::from_definition_flags(false, false, true, false, false);

    let pformat = render_subagent_system_prompt(
        workspace.path(),
        "round18-model",
        &[0, 99],
        &parent_tools,
        &extra_tools,
        "archetype body",
        options,
        ToolCallFormat::PFormat,
        &[],
    );
    assert!(pformat.contains("archetype body"));
    assert!(pformat.contains("### PROFILE.md"));
    assert!(pformat.contains("### MEMORY.md"));
    assert!(pformat.contains("Call as: `alpha[message|zeta]`"));
    assert!(pformat.contains("Call as: `extra[message|zeta]`"));
    assert!(pformat.contains("## Safety"));

    let json_prompt = render_subagent_system_prompt(
        workspace.path(),
        "round18-model",
        &[0],
        &parent_tools,
        &[],
        "",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Json,
        &[],
    );
    assert!(json_prompt.contains("Parameters:"));

    let native_prompt = render_subagent_system_prompt(
        workspace.path(),
        "round18-model",
        &[0],
        &parent_tools,
        &[],
        "",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Native,
        &[],
    );
    assert!(!native_prompt.contains("## Tools"));
    assert!(native_prompt.contains("native tool-calling output"));
}

#[test]
fn agent_builder_validation_reports_each_required_component() {
    let provider = ScriptedProvider::new(vec![]);

    let err = match Agent::builder().build() {
        Ok(_) => panic!("builder without tools should fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("tools are required"));

    let err = match Agent::builder().tools(Vec::new()).build() {
        Ok(_) => panic!("builder without provider should fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("provider is required"));

    let err = match Agent::builder()
        .tools(Vec::new())
        .provider_arc(provider.clone())
        .build()
    {
        Ok(_) => panic!("builder without memory should fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("memory is required"));

    let err = match Agent::builder()
        .tools(Vec::new())
        .provider_arc(provider)
        .memory(Arc::new(StubMemory))
        .build()
    {
        Ok(_) => panic!("builder without dispatcher should fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("tool_dispatcher is required"));

    let agent = Agent::builder()
        .tools(vec![tool("echo"), tool("echo")])
        .provider_arc(ScriptedProvider::new(vec![]))
        .memory(Arc::new(StubMemory))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .visible_tool_names(HashSet::from(["echo".to_string()]))
        .agent_definition_name("round18/custom name")
        .build()
        .expect("complete builder should succeed");
    assert_eq!(agent.agent_definition_name(), "round18/custom name");
}

#[tokio::test]
async fn run_subagent_loads_workspace_prompt_runs_tool_and_returns_final() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir_all(workspace.path().join("agent/prompts"))?;
    std::fs::write(
        workspace.path().join("agent/prompts/worker.md"),
        "workspace prompt body",
    )?;
    std::fs::write(workspace.path().join("PROFILE.md"), "profile from disk")?;
    std::fs::write(workspace.path().join("MEMORY.md"), "memory from disk")?;
    let provider = ScriptedProvider::new(vec![
        tool_response("echo", json!({"message": "hello"})),
        text_response("final from subagent"),
    ]);
    let def = definition(PromptSource::File {
        path: "worker.md".to_string(),
    });

    let outcome = with_parent_context(
        parent(workspace.path().to_path_buf(), provider.clone()),
        async {
            run_subagent(
                &def,
                "do the deterministic thing",
                SubagentRunOptions {
                    task_id: Some("round18-task".to_string()),
                    context: Some("caller context".to_string()),
                    ..SubagentRunOptions::default()
                },
            )
            .await
        },
    )
    .await?;

    assert_eq!(outcome.output, "final from subagent");
    assert_eq!(outcome.iterations, 2);
    let requests = provider.requests();
    assert!(requests[0].contains("workspace prompt body"));
    assert!(requests[0].contains("parent memory survives when allowed"));
    assert!(requests[0].contains("caller context"));
    assert!(requests[1].contains("tool-output"));
    Ok(())
}

#[tokio::test]
async fn run_subagent_missing_file_falls_back_to_empty_prompt() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let provider = ScriptedProvider::new(vec![text_response("fallback ok")]);
    let def = definition(PromptSource::File {
        path: "missing.md".to_string(),
    });

    let outcome = with_parent_context(
        parent(workspace.path().to_path_buf(), provider.clone()),
        async { run_subagent(&def, "task", SubagentRunOptions::default()).await },
    )
    .await?;

    assert_eq!(outcome.output, "fallback ok");
    assert!(provider.requests()[0].contains("## Sub-agent Role Contract"));
    Ok(())
}

#[tokio::test]
async fn run_subagent_surfaces_provider_errors_and_can_be_cancelled() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let failing = ScriptedProvider::failing("round18 provider failure");
    let def = definition(PromptSource::Inline("inline prompt".to_string()));

    let result = with_parent_context(parent(workspace.path().to_path_buf(), failing), async {
        run_subagent(&def, "task", SubagentRunOptions::default()).await
    })
    .await;
    assert!(matches!(result, Err(SubagentRunError::Provider(_))));

    let slow = ScriptedProvider::delayed(Duration::from_secs(30));
    let slow_parent = parent(workspace.path().to_path_buf(), slow.clone());
    let slow_def = def.clone();
    let handle = tokio::spawn(async move {
        with_parent_context(slow_parent, async {
            run_subagent(&slow_def, "slow task", SubagentRunOptions::default()).await
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    let cancelled = handle.await;
    assert!(cancelled.is_err());
    assert!(
        !slow.requests().is_empty(),
        "provider request should have started before abort"
    );

    Ok(())
}
