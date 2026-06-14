use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::harness::definition::AgentTier;
use openhuman_core::openhuman::agent::harness::session::Agent;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, DefinitionSource, ModelSpec,
    ParentExecutionContext, PromptSource, SandboxMode, SubagentRunOptions, ToolScope,
};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::{
    render_ambient_environment, render_subagent_system_prompt, render_tools, render_user_files,
    ConnectedIntegration, CuratedMemoryPromptSnapshot, LearnedContextData, NamespaceSummary,
    PersonalityRosterEntry, PromptContext, PromptTool, SubagentRenderOptions, SystemPromptBuilder,
    ToolCallFormat, UserIdentity,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary as MemoryNamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolContent, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    native_tools: bool,
}

#[derive(Clone)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    tool_names: Vec<String>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
            native_tools: true,
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
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
        Ok(format!("checkpoint:{message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tool_names: request
                .tools
                .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
                .unwrap_or_default(),
        });
        self.responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("fallback final")))
    }
}

#[derive(Default)]
struct StubMemory {
    entries: Mutex<Vec<MemoryEntry>>,
}

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "round19-memory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        let mut entries = self.entries.lock();
        let id = format!("{namespace}:{key}:{}", entries.len());
        entries.push(MemoryEntry {
            id,
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-05-29T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: Some(0.9),
            taint: Default::default(),
        });
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(self.entries.lock().iter().take(limit).cloned().collect())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .find(|entry| entry.namespace.as_deref() == Some(namespace) && entry.key == key)
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .filter(|entry| namespace.is_none_or(|ns| entry.namespace.as_deref() == Some(ns)))
            .filter(|entry| category.is_none_or(|cat| &entry.category == cat))
            .filter(|entry| session_id.is_none_or(|sid| entry.session_id.as_deref() == Some(sid)))
            .cloned()
            .collect())
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool> {
        let mut entries = self.entries.lock();
        let before = entries.len();
        entries.retain(|entry| entry.namespace.as_deref() != Some(namespace) || entry.key != key);
        Ok(entries.len() != before)
    }

    async fn namespace_summaries(&self) -> Result<Vec<MemoryNamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.entries.lock().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct EchoTool {
    name: &'static str,
    permission: PermissionLevel,
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round19 deterministic echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "alpha": { "type": "string" },
                "zeta": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult {
            content: vec![ToolContent::Text {
                text: format!("echo:{args}"),
            }],
            is_error: false,
            markdown_formatted: Some(format!("**echo** `{args}`")),
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
}

fn tool(name: &'static str) -> Box<dyn Tool> {
    Box::new(EchoTool {
        name,
        permission: PermissionLevel::ReadOnly,
    })
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 11,
            output_tokens: 5,
            context_window: 8_192,
            cached_input_tokens: 3,
            charged_amount_usd: 0.002,
        }),
        reasoning_content: None,
    }
}

fn empty_response() -> ChatResponse {
    ChatResponse {
        text: None,
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    }
}

fn tool_response(id: &str, name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some("using tool".to_string()),
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            extra_content: None,
        }],
        usage: Some(UsageInfo {
            input_tokens: 7,
            output_tokens: 2,
            context_window: 8_192,
            cached_input_tokens: 1,
            charged_amount_usd: 0.001,
        }),
        reasoning_content: Some("because tool".to_string()),
    }
}

fn agent_config(max_tool_iterations: usize) -> AgentConfig {
    AgentConfig {
        max_tool_iterations,
        max_history_messages: 8,
        ..AgentConfig::default()
    }
}

fn build_agent(
    workspace: &Path,
    provider: Arc<ScriptedProvider>,
    tools: Vec<Box<dyn Tool>>,
) -> Result<Agent> {
    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(tools)
        .memory(Arc::new(StubMemory::default()))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .config(agent_config(3))
        .model_name("round19-model".to_string())
        .temperature(0.0)
        .workspace_dir(workspace.to_path_buf())
        .workflows(Vec::new())
        .auto_save(false)
        .event_context("round19-session", "round19-channel")
        .agent_definition_name("round19_agent")
        .omit_profile(true)
        .omit_memory_md(true)
        .explicit_preferences_enabled(false)
        .build()?;
    agent.set_connected_integrations(Vec::new());
    Ok(agent)
}

fn prompt_context<'a>(
    workspace: &'a Path,
    tools: &'a [PromptTool<'a>],
    visible: &'a HashSet<String>,
    learned: LearnedContextData,
    format: ToolCallFormat,
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: workspace,
        model_name: "round19-model",
        agent_id: "round19_agent",
        tools,
        workflows: &[],
        dispatcher_instructions: "dispatcher guidance",
        learned,
        visible_tool_names: visible,
        tool_call_format: format,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: Vec::new(),
    }
}

fn definition(max_result_chars: Option<usize>) -> AgentDefinition {
    AgentDefinition {
        id: "round19_worker".to_string(),
        when_to_use: "raw coverage worker".to_string(),
        display_name: Some("Round 19 Worker".to_string()),
        system_prompt: PromptSource::Inline("Worker prompt".to_string()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Wildcard,
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 2,
        iteration_policy: Default::default(),
        max_result_chars,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::Builtin,
    }
}

fn parent_context(workspace: PathBuf, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
    let tools = vec![tool("echo")];
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
        model_name: "round19-parent".to_string(),
        temperature: 0.0,
        workspace_dir: workspace,
        memory: Arc::new(StubMemory::default()),
        agent_config: agent_config(3),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory context".to_string())),
        session_id: "round19-parent-session".to_string(),
        channel: "round19-channel".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "1700000000_parent".to_string(),
        session_parent_prefix: Some("root-chain".to_string()),
        on_progress: None,
        run_queue: None,
    }
}

#[tokio::test]
async fn turn_rejects_empty_final_response_and_keeps_history_nonfinal() -> Result<()> {
    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![empty_response()]);
    let mut agent = build_agent(tmp.path(), provider, vec![tool("echo")])?;

    let err = agent.turn("return an empty response").await.unwrap_err();

    assert!(err.to_string().contains("empty response"));
    assert!(agent
        .history()
        .iter()
        .any(|message| matches!(message, openhuman_core::openhuman::inference::provider::ConversationMessage::Chat(chat) if chat.role == "user")));
    Ok(())
}

#[tokio::test]
async fn turn_dedups_visible_tool_specs_and_preserves_reasoning_metadata() -> Result<()> {
    let tmp = TempDir::new()?;
    let mut first = text_response("first final");
    first.reasoning_content = Some("private reasoning trace".to_string());
    let provider = ScriptedProvider::new(vec![first, text_response("second final")]);
    let mut agent = build_agent(
        tmp.path(),
        provider.clone(),
        vec![tool("echo"), tool("echo")],
    )?;

    assert_eq!("first final", agent.turn("first").await?);
    assert_eq!("second final", agent.turn("second").await?);

    let requests = provider.requests();
    assert_eq!(requests[0].tool_names, vec!["echo"]);
    assert!(requests[1].messages.iter().any(|message| message
        .extra_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("reasoning_content"))
        .and_then(serde_json::Value::as_str)
        == Some("private reasoning trace")));
    Ok(())
}

#[tokio::test]
async fn seed_resume_bounds_unknown_roles_and_drops_current_tail() -> Result<()> {
    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![text_response("resumed final")]);
    let mut agent = build_agent(tmp.path(), provider.clone(), vec![tool("echo")])?;

    agent.seed_resume_from_messages(
        vec![
            ("user".to_string(), "older question".to_string()),
            ("bot".to_string(), "unknown sender becomes user".to_string()),
            ("assistant".to_string(), "prior assistant".to_string()),
            ("user".to_string(), "current question".to_string()),
        ],
        "current question",
    )?;
    assert_eq!("resumed final", agent.turn("current question").await?);

    let first_request = provider.requests().remove(0);
    let sent = first_request
        .messages
        .iter()
        .map(|message| format!("{}:{}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sent.contains("user:unknown sender becomes user"));
    assert!(sent.contains("assistant:prior assistant"));
    assert_eq!(sent.matches("current question").count(), 1);
    Ok(())
}

#[tokio::test]
async fn builder_reports_missing_required_fields_in_validation_order() -> Result<()> {
    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![text_response("unused")]);

    let err = match Agent::builder().build() {
        Ok(_) => panic!("builder without tools should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("tools are required"));

    let err = match Agent::builder().tools(Vec::new()).build() {
        Ok(_) => panic!("builder without provider should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("provider is required"));

    let err = match Agent::builder()
        .tools(Vec::new())
        .provider_arc(provider)
        .workspace_dir(tmp.path().to_path_buf())
        .build()
    {
        Ok(_) => panic!("builder without memory should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("memory is required"));
    Ok(())
}

#[tokio::test]
async fn subagent_run_truncates_capped_final_output_after_parent_context_run() -> Result<()> {
    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![text_response("abcdef")]);
    let parent = parent_context(tmp.path().to_path_buf(), provider);

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &definition(Some(3)),
            "do a tiny task",
            SubagentRunOptions {
                task_id: Some("round19-task".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await
    })
    .await?;

    assert_eq!(outcome.output, "abc\n[...truncated]");
    assert_eq!(outcome.iterations, 1);
    Ok(())
}

#[tokio::test]
async fn subagent_repeated_unknown_tool_halts_with_root_cause() -> Result<()> {
    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![
        tool_response("call-1", "missing_tool", json!({"same": true})),
        tool_response("call-2", "missing_tool", json!({"same": true})),
        tool_response("call-3", "missing_tool", json!({"same": true})),
    ]);
    let parent = parent_context(tmp.path().to_path_buf(), provider);
    let mut def = definition(None);
    def.max_iterations = 3;

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &def,
            "repeat an unavailable tool",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await?;

    assert!(outcome.output.contains("repeating it will not help"));
    assert!(outcome
        .output
        .contains("tool 'missing_tool' is not available"));
    assert_eq!(outcome.iterations, 3);
    Ok(())
}

#[test]
fn prompt_builder_renders_dynamic_user_files_and_identity_branches() -> Result<()> {
    let tmp = TempDir::new()?;
    std::fs::write(tmp.path().join("PROFILE.md"), "Profile body")?;
    std::fs::write(tmp.path().join("MEMORY.md"), "Workspace memory body")?;
    let visible = HashSet::new();
    let tools = vec![PromptTool::with_schema(
        "echo",
        "Echo tool",
        json!({"type":"object","properties":{"zeta":{},"alpha":{}}}).to_string(),
    )];
    let mut learned = LearnedContextData::default();
    learned.reflections = vec!["  prefers concise updates  ".to_string(), " ".to_string()];
    learned.tree_root_summaries = vec![NamespaceSummary {
        namespace: "work".to_string(),
        body: "Durable memory".to_string(),
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-05-20T00:00:00Z")?
            .with_timezone(&chrono::Utc),
    }];
    let mut ctx = prompt_context(
        tmp.path(),
        &tools,
        &visible,
        learned,
        ToolCallFormat::PFormat,
    );
    ctx.include_profile = true;
    ctx.include_memory_md = true;
    ctx.curated_snapshot = Some(Arc::new(CuratedMemoryPromptSnapshot {
        memory: "Curated memory".to_string(),
        user: "Curated user".to_string(),
    }));
    ctx.user_identity = Some(UserIdentity {
        id: Some(" user\nid ".to_string()),
        name: Some(" Ada\r Lovelace ".to_string()),
        email: Some(" ada@example.test ".to_string()),
    });
    ctx.personality_roster = vec![PersonalityRosterEntry {
        id: "critic".to_string(),
        name: "Critic".to_string(),
        description: "Reviews plans".to_string(),
        memory_summary: Some("x".repeat(240)),
    }];

    let prompt = SystemPromptBuilder::from_dynamic(|ctx| {
        let mut out = String::new();
        out.push_str(&render_user_files(ctx)?);
        out.push_str(&render_tools(ctx)?);
        out.push_str(&render_ambient_environment(ctx)?);
        Ok(out)
    })
    .build(&ctx)?;

    assert!(prompt.contains("### PROFILE.md"));
    assert!(prompt.contains("Curated memory"));
    assert!(prompt.contains("Curated user"));
    assert!(prompt.contains("echo[alpha|zeta]"));
    assert!(prompt.contains("- name: Ada Lovelace"));
    assert!(prompt.contains("- id: user id"));
    assert!(prompt.contains("## Current Date & Time"));

    ctx.curated_snapshot = None;
    ctx.personality_memory_md = Some("Personality memory".to_string());
    let user_files = render_user_files(&ctx)?;
    assert!(user_files.contains("Personality memory"));
    assert!(!user_files.contains("Workspace memory body"));
    Ok(())
}

#[test]
fn subagent_prompt_renderer_handles_formats_caps_and_stale_tool_indices() -> Result<()> {
    let tmp = TempDir::new()?;
    std::fs::write(tmp.path().join("PROFILE.md"), "P".repeat(2_100))?;
    std::fs::write(tmp.path().join("MEMORY.md"), "Memory file")?;
    let parent_tools = vec![tool("echo")];
    let options = SubagentRenderOptions {
        include_safety_preamble: true,
        include_identity: false,
        include_skills_catalog: false,
        include_profile: true,
        include_memory_md: true,
    };
    let connected = vec![ConnectedIntegration {
        toolkit: "gmail".to_string(),
        description: "Mail".to_string(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    }];

    let json_prompt = render_subagent_system_prompt(
        tmp.path(),
        "round19-model",
        &[0, 99],
        &parent_tools,
        &[tool("extra")],
        "Archetype",
        options,
        ToolCallFormat::Json,
        &connected,
    );
    assert!(json_prompt.contains("Parameters:"));
    assert!(json_prompt.contains("extra"));
    assert!(json_prompt.contains("truncated at 2000 chars"));
    assert!(json_prompt.contains("## Safety"));
    assert!(json_prompt.contains("## Output style"));

    let native_prompt = render_subagent_system_prompt(
        tmp.path(),
        "round19-model",
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
    Ok(())
}
