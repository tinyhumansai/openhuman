use anyhow::Result;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use openhuman_core::openhuman::agent::debug::{dump_agent_prompt, DumpPromptOptions};
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::{
    render_ambient_environment, render_safety, render_subagent_system_prompt_with_format,
    render_tools, ConnectedIntegration, CuratedMemoryPromptSnapshot, LearnedContextData,
    NamespaceSummary as PromptNamespaceSummary, PersonalityRosterEntry, PersonalityRosterSection,
    PromptContext, PromptTool, SubagentRenderOptions, SystemPromptBuilder, ToolCallFormat,
    UserIdentity,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::subconscious::SourceChunk;
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use openhuman_core::openhuman::workflows::ops_types::Workflow;
use parking_lot::Mutex;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    tool_names: Vec<String>,
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<ChatResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    native_tools: bool,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from(responses)),
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
        Ok(self
            .responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| text_response("round26 fallback")))
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
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "round26-memory"
    }

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

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct Round26Tool {
    name: &'static str,
}

#[async_trait]
impl Tool for Round26Tool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round26 deterministic test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "alpha": { "type": "string" },
                "zeta": { "type": "integer" }
            }
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("round26 tool output"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 3,
            output_tokens: 2,
            context_window: 16_000,
            cached_input_tokens: 1,
            charged_amount_usd: 0.0001,
        }),
        reasoning_content: None,
    }
}

fn prompt_context<'a>(
    workspace: &'a std::path::Path,
    tools: &'a [PromptTool<'a>],
    visible: &'a HashSet<String>,
    format: ToolCallFormat,
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: workspace,
        model_name: "round26-model",
        agent_id: "round26-agent",
        tools,
        workflows: &[] as &[Workflow],
        dispatcher_instructions: "round26 dispatcher instructions",
        learned: LearnedContextData {
            reflections: vec![
                "  prefer concise status updates  ".to_string(),
                String::new(),
            ],
            tree_root_summaries: vec![
                PromptNamespaceSummary {
                    namespace: "projects".to_string(),
                    body: "Root memory summary for round26.".to_string(),
                    updated_at: Utc.with_ymd_and_hms(2026, 5, 28, 12, 0, 0).unwrap(),
                },
                PromptNamespaceSummary {
                    namespace: "empty".to_string(),
                    body: "   ".to_string(),
                    updated_at: Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0).unwrap(),
                },
            ],
            ..LearnedContextData::default()
        },
        visible_tool_names: visible,
        tool_call_format: format,
        connected_integrations: &[] as &[ConnectedIntegration],
        connected_identities_md: String::new(),
        include_profile: true,
        include_memory_md: true,
        curated_snapshot: Some(Arc::new(CuratedMemoryPromptSnapshot {
            memory: "curated memory snapshot round26".to_string(),
            user: "curated user snapshot round26".to_string(),
        })),
        user_identity: Some(UserIdentity {
            id: Some(" user-26 ".to_string()),
            name: Some("Round\nTwenty Six".to_string()),
            email: Some(" round26@example.test ".to_string()),
        }),
        personality_soul_md: Some("round26 personality soul override".to_string()),
        personality_memory_md: Some("round26 personality memory override".to_string()),
        personality_roster: vec![PersonalityRosterEntry {
            id: "analyst".to_string(),
            name: "Analyst".to_string(),
            description: "Checks cold prompt paths".to_string(),
            memory_summary: Some("x".repeat(240)),
        }],
    }
}

#[test]
fn prompt_renderers_cover_user_memory_identity_tools_and_subagent_variants() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("PROFILE.md"), "profile file round26")?;
    std::fs::write(
        workspace.path().join("MEMORY.md"),
        "workspace memory round26",
    )?;

    let schema = json!({
        "type": "object",
        "properties": {
            "zeta": { "type": "integer" },
            "alpha": { "type": "string" }
        }
    })
    .to_string();
    let tools = [PromptTool::with_schema(
        "round26_tool",
        "Prompt-rendered tool",
        schema,
    )];
    let mut visible = HashSet::new();
    visible.insert("round26_tool".to_string());
    let ctx = prompt_context(workspace.path(), &tools, &visible, ToolCallFormat::PFormat);

    let built = SystemPromptBuilder::with_defaults()
        .add_section(Box::new(PersonalityRosterSection))
        .with_reflection_context(vec![
            SourceChunk {
                ref_id: "entity:alpha".to_string(),
                kind: "entity".to_string(),
                content: "Resolved source chunk\nwith newline.".to_string(),
                metadata: json!({"hotness": 7}),
            },
            SourceChunk {
                ref_id: "missing:beta".to_string(),
                kind: "missing".to_string(),
                content: String::new(),
                metadata: json!({}),
            },
        ])
        .build(&ctx)?;

    assert!(built.contains("round26 personality soul override"));
    assert!(built.contains("### PROFILE.md"));
    assert!(built.contains("profile file round26"));
    assert!(built.contains("round26 personality memory override"));
    assert!(built.contains("## User Memory"));
    assert!(built.contains("projects (last updated 2026-05-28)"));
    assert!(built.contains("round26_tool[alpha|zeta]"));
    assert!(built.contains("## Memory context"));
    assert!(built.contains("Resolved source chunk with newline."));
    assert!(!built.contains("missing:beta"));
    assert!(built.contains("## Available Personalities"));
    assert!(built.contains("Recent context: "));

    let ambient = render_ambient_environment(&ctx)?;
    assert!(ambient.contains("## Runtime"));
    assert!(ambient.contains("- name: Round Twenty Six"));
    assert!(ambient.contains("- email: round26@example.test"));
    assert!(ambient.contains("## Current Date & Time"));

    let native_ctx = prompt_context(workspace.path(), &tools, &visible, ToolCallFormat::Native);
    let native_tools = render_tools(&native_ctx)?;
    assert_eq!(native_tools.trim(), "round26 dispatcher instructions");
    assert!(render_safety().contains("Prefer `trash` over `rm`"));

    let parent_tools: Vec<Box<dyn Tool>> = vec![Box::new(Round26Tool {
        name: "parent_tool",
    })];
    let extra_tools: Vec<Box<dyn Tool>> = vec![Box::new(Round26Tool { name: "extra_tool" })];
    let subagent_json = render_subagent_system_prompt_with_format(
        workspace.path(),
        "round26-model",
        &[0, 99],
        &parent_tools,
        &extra_tools,
        "Round26 archetype",
        SubagentRenderOptions {
            include_safety_preamble: true,
            include_identity: true,
            include_skills_catalog: false,
            include_profile: true,
            include_memory_md: true,
        },
        ToolCallFormat::Json,
        &[],
    );
    assert!(subagent_json.contains("Round26 archetype"));
    assert!(subagent_json.contains("### PROFILE.md"));
    assert!(subagent_json.contains("- **parent_tool**"));
    assert!(subagent_json.contains("- **extra_tool**"));
    assert!(subagent_json.contains("Parameters:"));
    assert!(subagent_json.contains("## Safety"));

    let subagent_native = render_subagent_system_prompt_with_format(
        workspace.path(),
        "round26-model",
        &[0],
        &parent_tools,
        &extra_tools,
        "Round26 archetype",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Native,
        &[],
    );
    assert!(!subagent_native.contains("## Tools"));
    assert!(subagent_native.contains("native tool-calling output"));

    Ok(())
}

#[tokio::test]
async fn builder_dedupes_visible_native_tools_and_seed_resume_bounds_history() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let provider = ScriptedProvider::new(vec![text_response("round26 resumed final")]);

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(Round26Tool {
            name: "round26_duplicate",
        }),
        Box::new(Round26Tool {
            name: "round26_duplicate",
        }),
        Box::new(Round26Tool {
            name: "round26_hidden",
        }),
    ];
    let mut visible = HashSet::new();
    visible.insert("round26_duplicate".to_string());

    let mut agent = Agent::builder()
        .provider_arc(provider.clone())
        .tools(tools)
        .visible_tool_names(visible)
        .memory(Arc::new(StubMemory))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace.path().to_path_buf())
        .event_context("round26-session", "round26-channel")
        .agent_definition_name("round26/orchestrator")
        .config(AgentConfig {
            max_history_messages: 4,
            ..AgentConfig::default()
        })
        .explicit_preferences_enabled(false)
        .build()?;

    let original_key = agent.session_key().to_string();
    agent.set_agent_definition_name("round26 renamed/agent");
    assert_ne!(original_key, agent.session_key());
    assert!(agent.session_key().ends_with("_round26_renamed_agent"));

    agent.seed_resume_from_messages(
        vec![
            ("user".to_string(), "old user one".to_string()),
            ("agent".to_string(), "old assistant one".to_string()),
            ("systemish".to_string(), "falls back to user".to_string()),
            ("assistant".to_string(), "old assistant two".to_string()),
            ("user".to_string(), "current message".to_string()),
        ],
        " current message ",
    )?;

    let answer = agent.run_single("current message").await?;
    assert_eq!(answer, "round26 resumed final");

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool_names, vec!["round26_duplicate"]);
    assert!(requests[0]
        .messages
        .iter()
        .any(|msg| msg.role == "assistant" && msg.content == "old assistant two"));
    assert!(requests[0]
        .messages
        .iter()
        .any(|msg| msg.role == "user" && msg.content == "falls back to user"));
    // The live turn's user message is stamped with the per-turn
    // `Current Date & Time:` line (#3602), so match by suffix rather than
    // exact equality — the dedup contract (exactly one "current message"
    // user turn after resume) still holds.
    assert_eq!(
        requests[0]
            .messages
            .iter()
            .filter(|msg| msg.role == "user" && msg.content.ends_with("current message"))
            .count(),
        1
    );

    Ok(())
}

#[tokio::test]
async fn debug_dump_integrations_agent_reports_missing_toolkit_without_network() -> Result<()> {
    let _env = env_lock();
    let workspace = tempfile::tempdir()?;
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", workspace.path());

    let err = dump_agent_prompt(DumpPromptOptions::new("integrations_agent"))
        .await
        .expect_err("integrations_agent needs an explicit toolkit");
    let message = err.to_string();
    assert!(message.contains("integrations_agent requires a `toolkit` argument"));
    assert!(message.contains("composio list_connection"));

    let mut options = DumpPromptOptions::new("integrations_agent");
    options.workspace_dir_override = Some(PathBuf::from(workspace.path()));
    options.model_override = Some("round26-debug-model".to_string());
    let err = dump_agent_prompt(options)
        .await
        .expect_err("missing toolkit should fail before any remote client call");
    assert!(err
        .to_string()
        .contains("integrations_agent requires a `toolkit` argument"));

    Ok(())
}
