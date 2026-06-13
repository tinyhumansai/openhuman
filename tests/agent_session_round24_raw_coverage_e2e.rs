use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::XmlToolDispatcher;
use openhuman_core::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use openhuman_core::openhuman::agent::memory_loader::MemoryLoader;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::{AgentConfig, ContextConfig};
use openhuman_core::openhuman::context::prompt::{
    ConnectedIntegration, LearnedContextData, PersonalityRosterEntry, PersonalityRosterSection,
    PromptContext, PromptSection, PromptTool, SubagentRenderOptions, SystemPromptBuilder,
    ToolCallFormat, UserIdentity, UserIdentitySection,
};
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderDelta, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{
    PermissionLevel, Tool, ToolContent, ToolResult, ToolScope as RuntimeToolScope,
};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use tempfile::TempDir;
use tokio::time::{sleep, Duration, Instant};

static NO_FILTER: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);

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
    tools_sent: bool,
    stream_was_requested: bool,
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_events: Vec<ProviderDelta>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
            stream_events: Vec::new(),
        })
    }

    fn with_stream(responses: Vec<ChatResponse>, stream_events: Vec<ProviderDelta>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
            stream_events,
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(
        &self,
    ) -> openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
        openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
            native_tool_calling: false,
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
        self.requests.lock().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tools_sent: request.tools.is_some(),
            stream_was_requested: request.stream.is_some(),
        });
        if let Some(stream) = request.stream {
            for event in &self.stream_events {
                stream.send(event.clone()).await.ok();
            }
        }
        self.responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("fallback final", None)))
    }
}

#[derive(Default)]
struct RecordingMemory {
    stores: Mutex<Vec<(String, String, String, MemoryCategory)>>,
}

impl RecordingMemory {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl Memory for RecordingMemory {
    fn name(&self) -> &str {
        "round24-recording-memory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        self.stores.lock().push((
            namespace.to_string(),
            key.to_string(),
            content.to_string(),
            category,
        ));
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(vec![MemoryEntry {
            id: "round24-pref".to_string(),
            key: "general".to_string(),
            content: "Prefer exact status labels.".to_string(),
            namespace: Some("user_pref_general".to_string()),
            category: MemoryCategory::Core,
            timestamp: "2026-05-30T00:00:00Z".to_string(),
            session_id: None,
            score: Some(0.98),
            taint: Default::default(),
        }]
        .into_iter()
        .take(limit)
        .collect())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let entries = match namespace {
            Some("learning_observations") => vec![entry(
                "obs",
                "learning_observations",
                "Observed: user likes brief answers.\nwith newline",
            )],
            Some("learning_patterns") => {
                vec![entry(
                    "pat",
                    "learning_patterns",
                    "Pattern: checks artifacts.",
                )]
            }
            Some("learning_reflections") => vec![entry(
                "reflection",
                "learning_reflections",
                "I want durable memory to stay concise.",
            )],
            _ => Vec::new(),
        };
        Ok(entries)
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.stores.lock().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct EmptyMemoryLoader;

#[async_trait]
impl MemoryLoader for EmptyMemoryLoader {
    async fn load_context(&self, _memory: &dyn Memory, _user_message: &str) -> Result<String> {
        Ok(String::new())
    }
}

struct Round24Tool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for Round24Tool {
    fn name(&self) -> &str {
        "round24_echo"
    }

    fn description(&self) -> &str {
        "round24 deterministic echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            content: vec![ToolContent::Text {
                text: format!(
                    "echoed:{}",
                    args.get("value")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("empty")
                ),
            }],
            is_error: false,
            markdown_formatted: None,
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn scope(&self) -> RuntimeToolScope {
        RuntimeToolScope::All
    }
}

struct RecordingHook {
    calls: Arc<AtomicUsize>,
    contexts: Arc<Mutex<Vec<TurnContext>>>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "round24-recording-hook"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> Result<()> {
        self.contexts.lock().push(ctx.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn text_response(text: &str, usage: Option<UsageInfo>) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage,
        reasoning_content: None,
    }
}

fn xml_tool_response(value: &str) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "before <tool_call>{{\"name\":\"round24_echo\",\"arguments\":{{\"value\":\"{value}\"}}}}</tool_call>"
        )),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 80,
            output_tokens: 12,
            context_window: 16_000,
            cached_input_tokens: 8,
            charged_amount_usd: 0.0002,
        }),
        reasoning_content: None,
    }
}

fn entry(key: &str, namespace: &str, content: &str) -> MemoryEntry {
    MemoryEntry {
        id: format!("{namespace}:{key}"),
        key: key.to_string(),
        content: content.to_string(),
        namespace: Some(namespace.to_string()),
        category: MemoryCategory::Custom(namespace.to_string()),
        timestamp: "2026-05-30T00:00:00Z".to_string(),
        session_id: None,
        score: Some(0.9),
        taint: Default::default(),
    }
}

fn workspace(label: &str) -> (TempDir, PathBuf) {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!(
            "agent-session-round24-{label}-{}",
            uuid::Uuid::new_v4()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let temp = TempDir::new_in(root.parent().unwrap()).unwrap();
    let path = temp.path().join(label);
    std::fs::create_dir_all(&path).unwrap();
    (temp, path)
}

fn prompt_ctx<'a>(
    workspace_dir: &'a Path,
    tools: &'a [PromptTool<'a>],
    learned: LearnedContextData,
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir,
        model_name: "round24-model",
        agent_id: "round24-agent",
        tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned,
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    }
}

#[tokio::test]
async fn max_iteration_checkpoint_uses_deterministic_fallback_and_hooks() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("checkpoint-fallback");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_contexts = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider::with_stream(
        vec![
            xml_tool_response("alpha"),
            text_response(
                "<tool_call>{\"name\":\"round24_echo\",\"arguments\":{\"value\":\"again\"}}</tool_call>",
                None,
            ),
        ],
        vec![ProviderDelta::TextDelta {
            delta: "checkpoint delta".to_string(),
        }],
    );

    let mut agent = Agent::builder()
        .provider_arc(provider.clone())
        .tools(vec![Box::new(Round24Tool {
            calls: calls.clone(),
        })])
        .memory(RecordingMemory::new())
        .memory_loader(Box::new(EmptyMemoryLoader))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path.clone())
        .event_context("round24-session", "round24-channel")
        .agent_definition_name("round24/orchestrator")
        .post_turn_hooks(vec![Arc::new(RecordingHook {
            calls: hook_calls.clone(),
            contexts: hook_contexts.clone(),
        })])
        .config(AgentConfig {
            max_tool_iterations: 1,
            max_history_messages: 8,
            ..AgentConfig::default()
        })
        .context_config(ContextConfig::default())
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap();
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
    agent.set_on_progress(Some(progress_tx));

    let answer = agent.turn("hit the cap").await.unwrap();

    assert!(answer.contains("I reached the tool-call limit for this turn (1 steps)"));
    // The unified TurnEngine digest uses `- round24_echo [ok]: ...` format (no backticks).
    assert!(answer.contains("round24_echo"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    wait_for_hook_calls(&hook_calls, 1).await;
    let contexts = hook_contexts.lock();
    assert_eq!(contexts[0].assistant_response, answer);
    assert_eq!(contexts[0].iteration_count, 1);
    assert_eq!(contexts[0].tool_calls.len(), 1);

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].tools_sent);
    assert!(
        !requests[1].tools_sent,
        "checkpoint call must disable tools"
    );
    assert!(requests[1].stream_was_requested);
    assert!(requests[1]
        .messages
        .last()
        .is_some_and(|message| message.content.contains("maximum number of tool calls")));

    let mut streamed = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        streamed.push(event);
    }
    assert!(streamed.iter().any(|event| matches!(
        event,
        openhuman_core::openhuman::agent::progress::AgentProgress::TextDelta {
            delta,
            iteration: 2
        } if delta == "checkpoint delta"
    )));
}

#[tokio::test]
async fn builder_validation_and_system_prompt_cover_defaults_and_learning() {
    let _env = env_lock();
    let missing_tools = match Agent::builder().build() {
        Ok(_) => panic!("builder without tools should fail"),
        Err(err) => err,
    };
    assert!(missing_tools.to_string().contains("tools are required"));

    let (_temp, workspace_path) = workspace("builder-prompt");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    std::fs::write(workspace_path.join("PROFILE.md"), "Round24 profile").unwrap();
    std::fs::write(workspace_path.join("MEMORY.md"), "Round24 memory").unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let memory = RecordingMemory::new();
    let provider = ScriptedProvider::new(vec![text_response("learned final", None)]);
    let mut agent = Agent::builder()
        .provider_arc(provider.clone())
        .tools(vec![Box::new(Round24Tool { calls })])
        .memory(memory)
        .memory_loader(Box::new(EmptyMemoryLoader))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("round24-prompt-session", "round24-prompt-channel")
        .agent_definition_name("round24 prompt/name")
        .learning_enabled(true)
        .explicit_preferences_enabled(true)
        .omit_profile(false)
        .omit_memory_md(false)
        .unified_compaction_enabled(true)
        .build()
        .unwrap();

    let answer = agent.turn("build the learned prompt").await.unwrap();
    assert_eq!(answer, "learned final");
    let requests = provider.requests();
    let system_prompt = requests[0]
        .messages
        .iter()
        .find(|message| message.role == "system")
        .expect("first turn should send a system prompt");
    assert!(system_prompt.content.contains("Round24 profile"));
    assert!(system_prompt.content.contains("Round24 memory"));
    assert!(system_prompt.content.contains("round24_echo"));
    assert!(system_prompt.content.contains("## Tool Use Protocol"));
}

#[test]
fn prompt_sections_cover_dynamic_roster_identity_and_subagent_edges() {
    let (_temp, workspace_path) = workspace("prompt-sections");
    std::fs::write(workspace_path.join("SOUL.md"), "# Soul\nworkspace soul").unwrap();
    std::fs::write(
        workspace_path.join("IDENTITY.md"),
        "# Identity\nworkspace identity",
    )
    .unwrap();
    std::fs::write(workspace_path.join("MEMORY.md"), "workspace memory").unwrap();

    let prompt_tools = vec![PromptTool::with_schema(
        "round24_prompt_tool",
        "prompt visible tool",
        json!({
            "type": "object",
            "properties": {
                "beta": { "type": "string" },
                "alpha": { "type": "string" }
            }
        })
        .to_string(),
    )];
    let ctx = prompt_ctx(
        &workspace_path,
        &prompt_tools,
        LearnedContextData::default(),
    );
    let dynamic = SystemPromptBuilder::from_dynamic(|ctx| {
        Ok(format!(
            "dynamic for {} in {}",
            ctx.agent_id, ctx.model_name
        ))
    })
    .add_section(Box::new(UserIdentitySection));
    let rendered_dynamic = dynamic
        .build(&PromptContext {
            user_identity: Some(UserIdentity {
                id: Some("id\n42".to_string()),
                name: Some("Ada\r Lovelace".to_string()),
                email: Some("ada@example.com".to_string()),
            }),
            ..ctx
        })
        .unwrap();
    assert!(rendered_dynamic.contains("dynamic for round24-agent"));
    assert!(rendered_dynamic.contains("- id: id 42"));
    assert!(rendered_dynamic.contains("- name: Ada Lovelace"));

    let roster = PersonalityRosterSection
        .build(&PromptContext {
            personality_roster: vec![PersonalityRosterEntry {
                id: "analyst".to_string(),
                name: "Analyst".to_string(),
                description: "Finds evidence.".to_string(),
                memory_summary: Some(format!("{} tail", "x".repeat(240))),
            }],
            ..prompt_ctx(
                &workspace_path,
                &prompt_tools,
                LearnedContextData::default(),
            )
        })
        .unwrap();
    assert!(roster.contains("## Available Personalities"));
    assert!(roster.contains("Analyst"));
    assert!(roster.contains("Recent context:"));

    let parent_tools: Vec<Box<dyn Tool>> = vec![Box::new(Round24Tool {
        calls: Arc::new(AtomicUsize::new(0)),
    })];
    let subagent_json = openhuman_core::openhuman::context::prompt::render_subagent_system_prompt(
        &workspace_path,
        "round24-model",
        &[999, 0],
        &parent_tools,
        &[],
        "Subagent archetype",
        SubagentRenderOptions {
            include_identity: true,
            include_safety_preamble: true,
            include_skills_catalog: false,
            include_profile: false,
            include_memory_md: true,
        },
        ToolCallFormat::Json,
        &[] as &[ConnectedIntegration],
    );
    assert!(subagent_json.contains("Subagent archetype"));
    assert!(subagent_json.contains("workspace soul"));
    assert!(subagent_json.contains("### MEMORY.md"));
    assert!(subagent_json.contains("Parameters:"));
    assert!(subagent_json.contains("## Safety"));

    let final_body = SystemPromptBuilder::from_final_body("already composed".to_string())
        .insert_section_before("missing", Box::new(PersonalityRosterSection))
        .build(&PromptContext {
            personality_roster: vec![PersonalityRosterEntry {
                id: "coach".to_string(),
                name: "Coach".to_string(),
                description: "Keeps work focused.".to_string(),
                memory_summary: None,
            }],
            ..prompt_ctx(
                &workspace_path,
                &prompt_tools,
                LearnedContextData::default(),
            )
        })
        .unwrap();
    assert!(final_body.starts_with("already composed"));
    assert!(final_body.contains("Coach"));
}

async fn wait_for_hook_calls(calls: &AtomicUsize, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let actual = calls.load(Ordering::SeqCst);
        if actual >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for hook calls; expected {expected}, got {actual}"
        );
        sleep(Duration::from_millis(20)).await;
    }
}
