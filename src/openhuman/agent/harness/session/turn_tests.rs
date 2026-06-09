use super::*;
use crate::core::event_bus::{global, init_global, DomainEvent};
use crate::openhuman::agent::dispatcher::XmlToolDispatcher;
use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::agent::memory_loader::MemoryLoader;
use crate::openhuman::agent::tool_policy::{
    GeneratedToolRuntimeContext, GeneratedToolRuntimeRisk, ToolPolicy, ToolPolicyDecision,
    ToolPolicyRequest,
};
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, ConversationMessage, Provider, ToolResultMessage,
    UsageInfo,
};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::ToolResult;
use crate::openhuman::tools::{PermissionLevel, Tool};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout, Duration};

struct DummyProvider;

#[async_trait]
impl Provider for DummyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("unused".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("unused".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })
    }
}

struct SequenceProvider {
    responses: AsyncMutex<Vec<anyhow::Result<ChatResponse>>>,
    requests: AsyncMutex<Vec<Vec<ChatMessage>>>,
}

#[async_trait]
impl Provider for SequenceProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("unused".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().await.push(request.messages.to_vec());
        self.responses.lock().await.remove(0)
    }
}

struct FixedMemoryLoader {
    context: String,
}

#[async_trait]
impl MemoryLoader for FixedMemoryLoader {
    async fn load_context(
        &self,
        _memory: &dyn Memory,
        _user_message: &str,
    ) -> anyhow::Result<String> {
        Ok(self.context.clone())
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("echo-output"))
    }
}

struct CronAddProbeTool;

#[async_trait]
impl Tool for CronAddProbeTool {
    fn name(&self) -> &str {
        "cron_add"
    }

    fn description(&self) -> &str {
        "cron add probe"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success(format!("cron_add_args={args}")))
    }
}

struct CountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counting"
    }

    fn description(&self) -> &str {
        "counting"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("counting-output"))
    }
}

struct DenyCountingPolicy;

#[async_trait]
impl ToolPolicy for DenyCountingPolicy {
    fn name(&self) -> &str {
        "deny_counting"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        assert_eq!(request.tool_name, "counting");
        assert_eq!(request.context.session_id, "turn-test-session");
        assert_eq!(request.context.channel, "turn-test-channel");
        assert_eq!(request.context.agent_definition_id, "main");
        assert_eq!(request.context.call_id, "policy-1");
        assert_eq!(request.context.iteration, 1);
        ToolPolicyDecision::deny("locked by test policy")
    }
}

struct LongTool;

#[async_trait]
impl Tool for LongTool {
    fn name(&self) -> &str {
        "long"
    }

    fn description(&self) -> &str {
        "long"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("x".repeat(800)))
    }
}

struct CountingWriteTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingWriteTool {
    fn name(&self) -> &str {
        "write_notes"
    }

    fn description(&self) -> &str {
        "write notes"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("write-output"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
}

struct GeneratedContextTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for GeneratedContextTool {
    fn name(&self) -> &str {
        "generated_send"
    }

    fn description(&self) -> &str {
        "generated send"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("generated-output"))
    }

    fn generated_runtime_context(
        &self,
        _args: &serde_json::Value,
    ) -> Option<GeneratedToolRuntimeContext> {
        Some(GeneratedToolRuntimeContext {
            provider_id: "mail.runtime".to_string(),
            capability_id: "email.send".to_string(),
            risk: GeneratedToolRuntimeRisk::ExternalWrite,
            source_digest: Some("sha256:abc".to_string()),
            approval_id: Some("approval-1".to_string()),
        })
    }
}

struct RequireGeneratedContextPolicy;

#[async_trait]
impl ToolPolicy for RequireGeneratedContextPolicy {
    fn name(&self) -> &str {
        "require_generated_context"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        let context = request
            .generated_tool
            .as_ref()
            .expect("generated tool context should be threaded");
        assert_eq!(context.provider_id, "mail.runtime");
        assert_eq!(context.capability_id, "email.send");
        assert_eq!(context.risk, GeneratedToolRuntimeRisk::ExternalWrite);
        assert_eq!(context.approval_id.as_deref(), Some("approval-1"));
        ToolPolicyDecision::require_approval("generated context requires approval")
    }
}

struct RecordingHook {
    calls: Arc<AsyncMutex<Vec<TurnContext>>>,
    notify: Arc<Notify>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "recording"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        self.calls.lock().await.push(ctx.clone());
        self.notify.notify_waiters();
        Ok(())
    }
}

fn make_agent(visible_tool_names: Option<HashSet<String>>) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut builder = Agent::builder()
        .provider(Box::new(DummyProvider))
        .tools(vec![Box::new(EchoTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .config(crate::openhuman::config::AgentConfig {
            max_history_messages: 3,
            ..crate::openhuman::config::AgentConfig::default()
        });

    if let Some(names) = visible_tool_names {
        builder = builder.visible_tool_names(names);
    }

    builder.build().unwrap()
}

fn make_agent_with_builder(
    provider: Arc<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    memory_loader: Box<dyn MemoryLoader>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    config: crate::openhuman::config::AgentConfig,
    context_config: crate::openhuman::config::ContextConfig,
) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    Agent::builder()
        .provider_arc(provider)
        .tools(tools)
        .memory(mem)
        .memory_loader(memory_loader)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .post_turn_hooks(post_turn_hooks)
        .config(config)
        .context_config(context_config)
        .workspace_dir(workspace_path)
        .auto_save(true)
        .event_context("turn-test-session", "turn-test-channel")
        .build()
        .unwrap()
}

#[test]
fn trim_history_preserves_system_and_keeps_latest_non_system_entries() {
    let mut agent = make_agent(None);
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        ConversationMessage::Chat(ChatMessage::user("u1")),
        ConversationMessage::Chat(ChatMessage::assistant("a1")),
        ConversationMessage::Chat(ChatMessage::user("u2")),
        ConversationMessage::Chat(ChatMessage::assistant("a2")),
    ];

    agent.trim_history();

    assert_eq!(agent.history.len(), 4);
    assert!(matches!(&agent.history[0], ConversationMessage::Chat(msg) if msg.role == "system"));
    assert!(agent
        .history
        .iter()
        .all(|msg| !matches!(msg, ConversationMessage::Chat(chat) if chat.content == "u1")));
    assert!(agent
        .history
        .iter()
        .any(|msg| matches!(msg, ConversationMessage::Chat(chat) if chat.content == "a2")));
}

/// When the `max_history_messages` cap drops an `AssistantToolCalls` opener but
/// keeps its `ToolResults`, the window would otherwise open on an orphaned tool
/// result — serialized, a `tool` message with no preceding `tool_calls`, which
/// the provider rejects (the 400 that surfaces as "Something went wrong").
/// `trim_history` must snap past the orphan so the window starts on a clean turn.
#[test]
fn trim_history_snaps_past_orphaned_tool_results() {
    use crate::openhuman::inference::provider::{ToolCall, ToolResultMessage};

    let mut agent = make_agent(None); // max_history_messages = 3
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        // This opener is the oldest non-system entry, so the cap drops it...
        ConversationMessage::AssistantToolCalls {
            text: Some("calling".into()),
            tool_calls: vec![ToolCall {
                id: "call_x".into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            }],
            reasoning_content: None,
        },
        // ...orphaning this result at the head of the kept window.
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "call_x".into(),
            content: "result".into(),
        }]),
        ConversationMessage::Chat(ChatMessage::user("u2")),
        ConversationMessage::Chat(ChatMessage::assistant("a2")),
    ];

    agent.trim_history();

    assert!(
        !agent
            .history
            .iter()
            .any(|m| matches!(m, ConversationMessage::ToolResults(_))),
        "orphaned ToolResults must be dropped, not left at the window head"
    );
    assert!(
        matches!(agent.history.first(), Some(ConversationMessage::Chat(c)) if c.role == "system"),
        "system message is preserved"
    );
    // system + u2 + a2 (the bisected cycle is gone entirely).
    assert_eq!(agent.history.len(), 3);
}

#[test]
fn build_parent_context_and_sanitize_helpers_cover_snapshot_paths() {
    let mut agent = make_agent(None);
    agent.last_memory_context = Some("remember this".into());
    agent.workflows = vec![crate::openhuman::workflows::Workflow {
        name: "demo".into(),
        ..Default::default()
    }];

    let parent = agent.build_parent_execution_context();
    assert_eq!(parent.model_name, agent.model_name);
    assert_eq!(parent.temperature, agent.temperature);
    assert_eq!(parent.memory_context.as_deref(), Some("remember this"));
    assert_eq!(parent.session_id, "turn-test-session");
    assert_eq!(parent.channel, "turn-test-channel");
    assert_eq!(parent.workflows.len(), 1);

    assert_eq!(sanitize_learned_entry("   "), "");
    assert_eq!(
        sanitize_learned_entry("Bearer abcdef"),
        "[redacted: potential secret]"
    );
    let long = "x".repeat(500);
    assert_eq!(sanitize_learned_entry(&long).chars().count(), 200);
    assert!(collect_tree_root_summaries(agent.workspace_dir(), 8_000, 32_000).is_empty());
}

#[test]
fn collect_tree_root_summaries_maps_namespace_body_and_timestamp() {
    // #2944: the wrapper must carry the root node's `updated_at` from the
    // store tuple into the `NamespaceSummary` the prompt renderer stamps.
    use crate::openhuman::config::Config;
    use crate::openhuman::memory_tree::tree_runtime::store::write_node;
    use crate::openhuman::memory_tree::tree_runtime::types::{
        derive_parent_id, estimate_tokens, level_from_node_id, TreeNode,
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = Config {
        workspace_dir: workspace.clone(),
        ..Config::default()
    };

    let updated_at = chrono::DateTime::parse_from_rfc3339("2026-05-25T09:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let summary = "Distilled activities summary.";
    let node = TreeNode {
        node_id: "root".to_string(),
        namespace: "activities".to_string(),
        level: level_from_node_id("root"),
        parent_id: derive_parent_id("root"),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count: 0,
        created_at: updated_at,
        updated_at,
        metadata: None,
    };
    write_node(&config, &node).unwrap();

    let summaries = collect_tree_root_summaries(&workspace, 8_000, 32_000);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].namespace, "activities");
    assert_eq!(summaries[0].body, summary);
    assert_eq!(summaries[0].updated_at, updated_at);
}

#[tokio::test]
async fn transcript_roundtrip_work() {
    let mut agent = make_agent(None);

    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("done"),
    ];
    agent.persist_session_transcript(&messages, 10, 5, 3, 0.25, None);
    assert!(agent.session_transcript_path.is_some());

    let loaded = transcript::read_transcript(agent.session_transcript_path.as_ref().unwrap())
        .expect("transcript should be readable");
    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.meta.input_tokens, 10);

    let mut resumed = make_agent(None);
    resumed.workspace_dir = agent.workspace_dir.clone();
    resumed.agent_definition_name = agent.agent_definition_name.clone();
    resumed.try_load_session_transcript();
    assert_eq!(
        resumed.cached_transcript_messages.as_ref().map(|m| m.len()),
        Some(3)
    );
}

#[tokio::test]
async fn transcript_resume_is_bounded_by_max_history_messages() {
    let mut writer = make_agent(None);
    let mut messages = vec![ChatMessage::system("sys")];
    for idx in 0..8 {
        messages.push(ChatMessage::user(format!("u{idx}")));
        messages.push(ChatMessage::assistant(format!("a{idx}")));
    }
    writer.persist_session_transcript(&messages, 0, 0, 0, 0.0, None);

    let mut resumed = make_agent(None);
    resumed.workspace_dir = writer.workspace_dir.clone();
    resumed.agent_definition_name = writer.agent_definition_name.clone();
    resumed.config.max_history_messages = 5;
    resumed.try_load_session_transcript();

    let cached = resumed
        .cached_transcript_messages
        .as_ref()
        .expect("resume cache should be populated");
    assert_eq!(cached.len(), 5);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].content, "u6");
    assert_eq!(cached[2].content, "a6");
    assert_eq!(cached[3].content, "u7");
    assert_eq!(cached[4].content, "a7");
}

#[tokio::test]
async fn execute_tool_call_blocks_invisible_tool_and_emits_events() {
    let _ = init_global(64);
    let events = Arc::new(AsyncMutex::new(Vec::<DomainEvent>::new()));
    let events_handler = Arc::clone(&events);
    let _handle = global().unwrap().on("turn-events-test", move |event| {
        let events = Arc::clone(&events_handler);
        let cloned = event.clone();
        Box::pin(async move {
            events.lock().await.push(cloned);
        })
    });

    let mut visible = HashSet::new();
    visible.insert("other".to_string());
    let agent = make_agent(Some(visible));
    let call = ParsedToolCall {
        name: "echo".into(),
        arguments: serde_json::json!({}),
        tool_call_id: Some("tc-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("not available to this agent"));
    assert_eq!(record.name, "echo");
    assert!(!record.success);

    sleep(Duration::from_millis(20)).await;
    let captured = events.lock().await;
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::ToolExecutionStarted { tool_name, session_id }
            if tool_name == "echo" && session_id == "turn-test-session"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::ToolExecutionCompleted {
            tool_name,
            session_id,
            success,
            ..
        } if tool_name == "echo" && session_id == "turn-test-session" && !success
    )));
}

#[tokio::test]
async fn execute_tool_call_reports_unknown_tool() {
    let agent = make_agent(None);
    let call = ParsedToolCall {
        name: "missing".into(),
        arguments: serde_json::json!({}),
        tool_call_id: None,
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("Unknown tool: missing"));
    assert_eq!(record.name, "missing");
    assert!(!record.success);
}

#[tokio::test]
async fn execute_tool_call_rewrites_legacy_run_skill_for_builtin_cron_tools() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let agent = make_agent_with_builder(
        provider,
        vec![Box::new(CronAddProbeTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );
    let call = ParsedToolCall {
        name: "run_skill".into(),
        arguments: serde_json::json!({
            "skill_id": "cron_add",
            "inputs": {
              "name": "water-reminder",
              "schedule": { "kind": "every", "every_ms": 60000 },
              "job_type": "agent",
              "prompt": "remind me to drink water"
            }
        }),
        tool_call_id: Some("tc-run-skill-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(result.success, "{}", result.output);
    assert_eq!(result.name, "cron_add");
    assert_eq!(record.name, "cron_add");
    assert!(result.output.contains("\"every_ms\":60000"));
}

#[tokio::test]
async fn execute_tool_call_rewrites_run_workflow_for_builtin_cron_tools() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let agent = make_agent_with_builder(
        provider,
        vec![Box::new(CronAddProbeTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );
    // Current form: `run_workflow({workflow_id: "<builtin>", inputs})`.
    let call = ParsedToolCall {
        name: "run_workflow".into(),
        arguments: serde_json::json!({
            "workflow_id": "cron_add",
            "inputs": {
              "name": "water-reminder",
              "schedule": { "kind": "every", "every_ms": 60000 },
              "job_type": "agent",
              "prompt": "remind me to drink water"
            }
        }),
        tool_call_id: Some("tc-run-workflow-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(result.success, "{}", result.output);
    assert_eq!(result.name, "cron_add");
    assert_eq!(record.name, "cron_add");
    assert!(result.output.contains("\"every_ms\":60000"));
}

#[tokio::test]
async fn execute_tool_call_denies_tool_above_channel_permission() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let mut config = crate::openhuman::config::AgentConfig::default();
    config
        .channel_permissions
        .insert("turn-test-channel".into(), "read_only".into());
    let agent = make_agent_with_builder(
        provider,
        vec![Box::new(CountingWriteTool {
            calls: Arc::clone(&calls),
        })],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );
    let call = ParsedToolCall {
        name: "write_notes".into(),
        arguments: serde_json::json!({"text":"hello"}),
        tool_call_id: Some("write-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;

    assert!(!result.success);
    assert!(result.output.contains("blocked by tool policy"));
    assert_eq!(record.name, "write_notes");
    assert!(!record.success);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn system_prompt_includes_tool_policy_boundary() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let mut config = crate::openhuman::config::AgentConfig::default();
    config
        .channel_permissions
        .insert("turn-test-channel".into(), "read_only".into());
    let agent = make_agent_with_builder(
        provider,
        vec![
            Box::new(EchoTool),
            Box::new(CountingWriteTool {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        ],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("prompt");

    assert!(prompt.contains("## Tool Policy Boundary"));
    assert!(prompt.contains("Allowed tools: echo"));
    assert!(prompt.contains("Restricted tools: 1 omitted by policy"));
    assert!(!prompt.contains("write_notes"));
}

#[test]
fn set_agent_definition_name_refreshes_tool_policy_identity() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let mut config = crate::openhuman::config::AgentConfig::default();
    config
        .channel_permissions
        .insert("turn-test-channel".into(), "read_only".into());
    let mut agent = make_agent_with_builder(
        provider,
        vec![
            Box::new(EchoTool),
            Box::new(CountingWriteTool {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        ],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    agent.set_agent_definition_name("renamed_agent");

    assert_eq!(agent.tool_policy_session.profile.agent_id, "renamed_agent");
    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("prompt");
    assert!(prompt.contains("Agent: renamed_agent"));
}

#[tokio::test]
async fn execute_tool_call_denies_by_policy_before_tool_runs() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));

    let agent = Agent::builder()
        .provider(Box::new(DummyProvider))
        .tools(vec![Box::new(CountingTool {
            calls: Arc::clone(&calls),
        })])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .tool_policy(Arc::new(DenyCountingPolicy))
        .build()
        .unwrap();
    let call = ParsedToolCall {
        name: "counting".into(),
        arguments: serde_json::json!({ "value": 1 }),
        tool_call_id: Some("policy-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("denied by policy 'deny_counting'"));
    assert!(result.output.contains("locked by test policy"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(record.name, "counting");
    assert!(!record.success);
}

#[tokio::test]
async fn execute_tool_call_threads_generated_tool_context_into_policy() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));

    let agent = Agent::builder()
        .provider(Box::new(DummyProvider))
        .tools(vec![Box::new(GeneratedContextTool {
            calls: Arc::clone(&calls),
        })])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .tool_policy(Arc::new(RequireGeneratedContextPolicy))
        .build()
        .unwrap();
    let call = ParsedToolCall {
        name: "generated_send".into(),
        arguments: serde_json::json!({ "value": 1 }),
        tool_call_id: Some("policy-generated-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(!result.success);
    assert!(result.output.contains("requires approval by policy"));
    assert!(result
        .output
        .contains("generated context requires approval"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(record.name, "generated_send");
    assert!(!record.success);
}

#[tokio::test]
async fn turn_runs_full_tool_cycle_with_context_and_hooks() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some(
                    "preface <tool_call>{\"name\":\"echo\",\"arguments\":{\"value\":1}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("final answer".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let hook_calls = Arc::new(AsyncMutex::new(Vec::<TurnContext>::new()));
    let hook_notify = Arc::new(Notify::new());
    let hooks: Vec<Arc<dyn PostTurnHook>> = vec![Arc::new(RecordingHook {
        calls: Arc::clone(&hook_calls),
        notify: Arc::clone(&hook_notify),
    })];

    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: "[Injected]\n".into(),
        }),
        hooks,
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let response = agent
        .turn("hello world")
        .await
        .expect("turn should succeed");
    assert_eq!(response, "final answer");
    assert!(agent.last_memory_context.as_deref() == Some("[Injected]\n"));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls {
            text, tool_calls, ..
        }
            if text.as_deref().is_some_and(|value| value.contains("preface")) && tool_calls.len() == 1
    )));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::Chat(chat) if chat.role == "assistant" && chat.content == "final answer"
    )));

    timeout(Duration::from_secs(1), async {
        loop {
            if !hook_calls.lock().await.is_empty() {
                break;
            }
            hook_notify.notified().await;
        }
    })
    .await
    .expect("hook should fire");

    let recorded_hooks = hook_calls.lock().await;
    assert_eq!(recorded_hooks.len(), 1);
    assert_eq!(recorded_hooks[0].assistant_response, "final answer");
    assert_eq!(recorded_hooks[0].iteration_count, 2);
    assert_eq!(recorded_hooks[0].tool_calls.len(), 1);
    assert_eq!(recorded_hooks[0].tool_calls[0].name, "echo");
    drop(recorded_hooks);

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0][0].role, "system");
    assert!(requests[0][1].content.contains("[Injected]"));
    assert!(requests[0][1].content.contains("hello world"));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "assistant" && msg.content.contains("preface")));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]")));
}

#[tokio::test]
async fn turn_triggers_configured_memory_agent_before_parent_prompt() {
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("built-in agent definitions should load");
    assert!(
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .and_then(|registry| registry.get("agent_memory"))
            .is_some()
    );

    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("memory context: user prefers concise Rust changes".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("parent final".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory(&memory_cfg, &workspace_path).unwrap(),
    );

    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(vec![Box::new(EchoTool)])
        .memory(mem)
        .memory_loader(Box::new(FixedMemoryLoader {
            context: String::new(),
        }))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .config(crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        })
        .workspace_dir(workspace_path)
        .auto_save(false)
        .event_context("turn-test-session", "turn-test-channel")
        .trigger_memory_agent(
            crate::openhuman::agent::harness::definition::TriggerMemoryAgent::Always,
        )
        .build()
        .unwrap();
    assert_eq!(
        agent.trigger_memory_agent,
        crate::openhuman::agent::harness::definition::TriggerMemoryAgent::Always
    );

    let response = agent
        .turn("Implement the memory trigger.")
        .await
        .expect("turn should succeed");
    assert_eq!(response, "parent final");

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].iter().any(|msg| {
        msg.role == "user" && msg.content.contains("Implement the memory trigger.")
    }));
    assert!(requests[1].iter().any(|msg| {
        msg.role == "user"
            && msg.content.contains("## Memory agent context")
            && msg
                .content
                .contains("memory context: user prefers concise Rust changes")
            && msg.content.contains("Implement the memory trigger.")
    }));
}

#[tokio::test]
async fn turn_uses_cached_transcript_prefix_on_first_iteration() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("cached-final".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn Provider> = provider_impl.clone();
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );
    agent.cached_transcript_messages = Some(vec![
        ChatMessage::system("cached-system"),
        ChatMessage::assistant("cached-assistant"),
    ]);

    let response = agent.turn("fresh").await.expect("turn should succeed");
    assert_eq!(response, "cached-final");
    assert!(agent.cached_transcript_messages.is_none());

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].len(), 3);
    assert_eq!(requests[0][0].content, "cached-system");
    assert_eq!(requests[0][1].content, "cached-assistant");
    assert_eq!(requests[0][2].role, "user");
    assert_eq!(requests[0][2].content, "fresh");
}

#[tokio::test]
async fn turn_emits_checkpoint_when_max_tool_iterations_are_exceeded() {
    // First response forces a tool call (consuming the single allowed
    // iteration); the second is the model-written checkpoint the harness
    // requests (tools disabled) once the cap is hit. The turn must NOT
    // error anymore — it returns a resumable checkpoint so the thread stays
    // well-formed and the user can continue on their next message
    // (bug-report-2026-05-26 A1).
    let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some(
                    "**Done so far:** ran echo.\n**Next steps:** I'll continue from here.".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("turn should emit a checkpoint at the iteration cap, not error");
    assert!(
        reply.contains("Next steps"),
        "checkpoint should summarize next steps, got: {reply}"
    );
    // The tool-call history from the capped iteration is preserved...
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls { tool_calls, .. } if tool_calls.len() == 1
    )));
    // ...and the transcript ends on a well-formed assistant message (the
    // checkpoint), never a dangling tool cycle — this is what stops the
    // next message from silently wedging the thread.
    assert!(
        matches!(
            agent.history.last(),
            Some(ConversationMessage::Chat(msg))
                if msg.role == "assistant" && msg.content.contains("Next steps")
        ),
        "history should end on the assistant checkpoint, got: {:?}",
        agent.history.last()
    );
}

#[tokio::test]
async fn turn_errors_on_empty_provider_response() {
    // A completion with no text and no tool calls is never a valid final
    // answer — surface it as an error instead of accepting a blank reply,
    // which previously rendered as silence and wedged the thread
    // (bug-report-2026-05-26 A1, defect B).
    let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );

    let err = agent
        .turn("hello")
        .await
        .expect_err("an empty provider response should surface as an error");
    assert!(
        err.to_string().contains("empty response"),
        "expected an empty-response error, got: {err}"
    );
}

#[tokio::test]
async fn turn_checkpoint_falls_back_to_deterministic_summary_when_model_summary_empty() {
    // Tool call consumes the single iteration; the checkpoint request then
    // comes back empty. The harness must fall back to a deterministic
    // done/next summary so the turn never returns blank — the safety net
    // that guarantees the thread can't re-wedge (bug-report-2026-05-26 A1).
    let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("empty model checkpoint should fall back, not error");
    assert!(
        reply.contains("tool-call limit"),
        "deterministic fallback summary expected, got: {reply}"
    );
    assert!(
        reply.contains("echo"),
        "fallback should list the tool that ran, got: {reply}"
    );
}

#[tokio::test]
async fn turn_checkpoint_usage_is_folded_into_transcript_accounting() {
    // The extra checkpoint provider call costs tokens; those must land in
    // the persisted transcript's cumulative accounting rather than being
    // silently dropped (CodeRabbit review on bug-report-2026-05-26 A1).
    let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Tool iteration — provider reports no usage.
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Checkpoint call — reports usage that must be accounted for.
            Ok(ChatResponse {
                text: Some("**Done so far:** ran echo.\n**Next steps:** continue.".into()),
                tool_calls: vec![],
                usage: Some(UsageInfo {
                    input_tokens: 11,
                    output_tokens: 4,
                    cached_input_tokens: 2,
                    charged_amount_usd: 0.05,
                    ..UsageInfo::default()
                }),
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    agent
        .turn("hello")
        .await
        .expect("turn should emit a checkpoint at the iteration cap");

    let transcript = transcript::read_transcript(
        agent
            .session_transcript_path
            .as_ref()
            .expect("checkpoint turn should persist a transcript"),
    )
    .expect("transcript should be readable");
    // Only the checkpoint call reported usage, so the turn totals must equal
    // exactly its numbers — proof the extra call is accounted for, not lost.
    assert_eq!(
        transcript.meta.input_tokens, 11,
        "checkpoint input tokens should be folded into the turn total"
    );
    assert_eq!(transcript.meta.output_tokens, 4);
    assert_eq!(transcript.meta.cached_input_tokens, 2);
}

#[tokio::test]
async fn execute_tool_call_applies_inline_result_budget() {
    let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
    let agent = make_agent_with_builder(
        provider,
        vec![Box::new(LongTool)],
        Box::new(FixedMemoryLoader {
            context: String::new(),
        }),
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig {
            tool_result_budget_bytes: 300,
            ..crate::openhuman::config::ContextConfig::default()
        },
    );
    let call = ParsedToolCall {
        name: "long".into(),
        arguments: serde_json::json!({}),
        tool_call_id: Some("long-1".into()),
    };

    let (result, record) = agent.execute_tool_call(&call, 0).await;
    assert!(result.success);
    assert!(result.output.contains("truncated by tool_result_budget"));
    assert!(record.output_summary.starts_with("long: ok ("));
}

// ── Explicit-preferences narrow path ──────────────────────────────────────────
//
// These tests verify that `fetch_learned_context` correctly handles the three
// flag combinations:
//  1. both flags off   → empty context
//  2. explicit_preferences_enabled=true, learning_enabled=false
//     → only general user_pref entries returned, no inference data
//  3. learning_enabled=true  → full path (existing tests cover this; we only
//     verify that explicit entries are included as well)
//
// We use the real `UnifiedMemory` backend (sqlite) so the list/store round-trip
// is exercised end-to-end without mocking the memory layer.

fn make_agent_with_memory(
    memory: Arc<dyn Memory>,
    workspace_dir: std::path::PathBuf,
    learning_enabled: bool,
    explicit_preferences_enabled: bool,
) -> Agent {
    Agent::builder()
        .provider(Box::new(DummyProvider))
        .tools(vec![])
        .memory(memory)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_dir)
        .event_context("pref-test-session", "pref-test-channel")
        .learning_enabled(learning_enabled)
        .explicit_preferences_enabled(explicit_preferences_enabled)
        .build()
        .unwrap()
}

fn make_real_memory(workspace: &std::path::Path) -> Arc<dyn Memory> {
    use crate::openhuman::embeddings::NoopEmbedding;
    use crate::openhuman::memory_store::UnifiedMemory;
    Arc::new(UnifiedMemory::new(workspace, Arc::new(NoopEmbedding), None).unwrap())
}

#[tokio::test]
async fn fetch_learned_context_returns_empty_when_both_flags_off() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem = make_real_memory(tmp.path());

    // Store a pinned preference so we can verify it is NOT returned.
    mem.store(
        "user_profile",
        "pinned/tooling/package_manager",
        "[pinned] (class=tooling) package_manager: pnpm",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let agent = make_agent_with_memory(
        mem,
        tmp.path().to_path_buf(),
        false, // learning_enabled
        false, // explicit_preferences_enabled
    );

    let learned = agent.fetch_learned_context().await;

    assert!(
        learned.user_profile.is_empty(),
        "both flags off: user_profile must be empty, got {:?}",
        learned.user_profile
    );
    assert!(learned.observations.is_empty());
    assert!(learned.patterns.is_empty());
    assert!(learned.reflections.is_empty());
}

#[tokio::test]
async fn fetch_learned_context_returns_general_prefs_when_explicit_flag_on_learning_off() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem = make_real_memory(tmp.path());

    // Store two general preferences in the two-lane store (where save_preference
    // writes them). The explicit path now reads `user_pref_general`, not the
    // legacy `user_profile` pinned namespace.
    mem.store(
        crate::openhuman::memory::preferences::USER_PREF_GENERAL_NAMESPACE,
        "package_manager",
        "Use pnpm for package management.",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();
    mem.store(
        crate::openhuman::memory::preferences::USER_PREF_GENERAL_NAMESPACE,
        "verbosity",
        "Keep replies terse.",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let agent = make_agent_with_memory(
        mem,
        tmp.path().to_path_buf(),
        false, // learning_enabled — full inference stack OFF
        true,  // explicit_preferences_enabled — narrow path ON
    );

    let learned = agent.fetch_learned_context().await;

    assert_eq!(
        learned.user_profile.len(),
        2,
        "explicit flag on, learning off: expected 2 general preferences, got: {:?}",
        learned.user_profile
    );
    assert!(
        learned.user_profile.iter().any(|s| s.contains("pnpm")),
        "package_manager preference value must appear in user_profile: {:?}",
        learned.user_profile
    );
    assert!(
        learned.user_profile.iter().any(|s| s.contains("terse")),
        "verbosity preference value must appear in user_profile: {:?}",
        learned.user_profile
    );
    // Inference-derived data must remain empty — the stack was NOT engaged.
    assert!(
        learned.observations.is_empty(),
        "observations must be empty when learning_enabled=false"
    );
    assert!(
        learned.patterns.is_empty(),
        "patterns must be empty when learning_enabled=false"
    );
    assert!(
        learned.reflections.is_empty(),
        "reflections must be empty when learning_enabled=false"
    );
}

#[tokio::test]
async fn fetch_learned_context_explicit_flag_off_learning_off_returns_empty_even_with_stored_prefs()
{
    let tmp = tempfile::TempDir::new().unwrap();
    let mem = make_real_memory(tmp.path());

    mem.store(
        "user_profile",
        "pinned/style/tone",
        "[pinned] (class=style) tone: formal",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let agent = make_agent_with_memory(
        mem,
        tmp.path().to_path_buf(),
        false, // learning_enabled
        false, // explicit_preferences_enabled — both off
    );

    let learned = agent.fetch_learned_context().await;
    assert!(
        learned.user_profile.is_empty(),
        "both flags off: user_profile must be empty even when prefs exist, got: {:?}",
        learned.user_profile
    );
}

#[tokio::test]
async fn fetch_learned_context_loads_general_prefs_when_learning_enabled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem = make_real_memory(tmp.path());
    mem.store(
        crate::openhuman::memory::preferences::USER_PREF_GENERAL_NAMESPACE,
        "tone",
        "Be concise and direct.",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    // learning_enabled=true → full path, which now also sources standing prefs
    // from the explicit user_pref_general store (inferred facets are demoted, so
    // they are no longer injected as ground truth).
    let agent = make_agent_with_memory(mem, tmp.path().to_path_buf(), true, true);
    let learned = agent.fetch_learned_context().await;
    assert!(
        learned.user_profile.iter().any(|s| s.contains("concise")),
        "learning path must inject explicit general prefs into user_profile: {:?}",
        learned.user_profile
    );
}

// ── assistant_message_has_tool_calls — TAURI-RUST-7 envelope check ─────

#[test]
fn assistant_message_has_tool_calls_detects_native_envelope() {
    let body = serde_json::json!({
        "content": "calling tool",
        "tool_calls": [{
            "id": "tc-1",
            "name": "shell",
            "arguments": "{}"
        }]
    })
    .to_string();
    let msg = ChatMessage::assistant(body);
    assert!(super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_non_assistant_role() {
    let body = serde_json::json!({
        "content": "x",
        "tool_calls": [{ "id": "tc-1", "name": "shell", "arguments": "{}" }]
    })
    .to_string();
    let msg = ChatMessage::user(body);
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_plain_text_reply() {
    // Most common positive case for the previous over-broad check: a plain
    // assistant reply whose text happens to mention `tool_calls`.
    let msg = ChatMessage::assistant("I considered using tool_calls but chose not to.");
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_envelope_without_content_field() {
    // A bare `{"tool_calls": [...]}` JSON in the content (no `content` field)
    // is not the envelope `dispatcher.rs` emits.
    let body = serde_json::json!({
        "tool_calls": [{ "id": "tc-1", "name": "shell", "arguments": "{}" }]
    })
    .to_string();
    let msg = ChatMessage::assistant(body);
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_empty_tool_calls_array() {
    let body = serde_json::json!({
        "content": "no tools",
        "tool_calls": []
    })
    .to_string();
    let msg = ChatMessage::assistant(body);
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_malformed_tool_call_items() {
    // tool_call object missing `id` — not the native envelope shape.
    let body_no_id = serde_json::json!({
        "content": "x",
        "tool_calls": [{ "name": "shell", "arguments": "{}" }]
    })
    .to_string();
    assert!(!super::assistant_message_has_tool_calls(
        &ChatMessage::assistant(body_no_id)
    ));

    // tool_call object missing `arguments` — also rejected.
    let body_no_args = serde_json::json!({
        "content": "x",
        "tool_calls": [{ "id": "tc-1", "name": "shell" }]
    })
    .to_string();
    assert!(!super::assistant_message_has_tool_calls(
        &ChatMessage::assistant(body_no_args)
    ));
}

#[test]
fn assistant_message_has_tool_calls_rejects_non_object_root() {
    // Content is a JSON array, not an object.
    let msg = ChatMessage::assistant(r#"["just", "an", "array"]"#.to_string());
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

#[test]
fn assistant_message_has_tool_calls_rejects_non_json_content() {
    // Plain prose that doesn't parse as JSON at all — early-returns false via
    // the `let Ok(value) = serde_json::from_str(...)` arm. Keeps the message
    // when the trailing-strip uses this helper.
    let msg = ChatMessage::assistant("Just a normal text reply, no JSON here.");
    assert!(!super::assistant_message_has_tool_calls(&msg));
}

// ── bound_cached_transcript_messages — TAURI-RUST-7 trailing-strip ─────
//
// `bound_cached_transcript_messages` operates on a `Vec<ChatMessage>` (the
// dispatcher-serialised wire format), so its detection runs through
// `assistant_message_has_tool_calls`. Verify the symmetric trailing-strip
// pops unpaired tool_calls envelopes while leaving plain assistant replies
// untouched.

fn tool_calls_envelope(id: &str) -> String {
    serde_json::json!({
        "content": "calling tool",
        "tool_calls": [{
            "id": id,
            "name": "shell",
            "arguments": "{}"
        }]
    })
    .to_string()
}

#[test]
fn bound_cached_transcript_messages_pops_trailing_tool_calls_envelope() {
    let agent = make_agent(None); // max_history_messages = 3
                                  // Need > max so the bound runs (early-returns when len <= max).
    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("u1"),
        ChatMessage::assistant("a1"),
        ChatMessage::user("u2"),
        ChatMessage::assistant(tool_calls_envelope("tc-trailing")),
    ];

    // With `max_history_messages = 3` and the leading `system` message,
    // `bound_cached_transcript_messages` keeps the last 2 non-system entries
    // — i.e. `[system, u2, trailing-envelope]`. After the envelope pop the
    // tail is `user("u2")`, not the dropped assistant message.
    let bounded = agent.bound_cached_transcript_messages(messages);
    assert!(
        bounded
            .last()
            .is_some_and(|m| m.role == "user" && m.content == "u2"),
        "trailing tool_calls envelope must be popped; expected user tail 'u2' — got tail role={:?} content={:?}",
        bounded.last().map(|m| m.role.as_str()),
        bounded.last().map(|m| m.content.as_str())
    );
    assert!(
        !bounded.iter().any(super::assistant_message_has_tool_calls),
        "no tool_calls envelope should survive the strip"
    );
}

#[test]
fn bound_cached_transcript_messages_leaves_plain_assistant_tail_intact() {
    let agent = make_agent(None); // max_history_messages = 3
    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("u1"),
        ChatMessage::assistant("a1"),
        ChatMessage::user("u2"),
        ChatMessage::assistant("plain text reply, no tool_calls"),
    ];

    let bounded = agent.bound_cached_transcript_messages(messages);
    let tail = bounded.last().expect("bounded transcript is non-empty");
    assert_eq!(tail.role, "assistant");
    assert_eq!(tail.content, "plain text reply, no tool_calls");
}

#[test]
fn bound_cached_transcript_messages_strips_multiple_trailing_envelopes() {
    // Defence-in-depth: if the cached transcript ends on multiple consecutive
    // unpaired tool_calls envelopes (e.g. two abortive turns), pop them all.
    let agent = make_agent(None);
    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("u1"),
        ChatMessage::assistant("a1"),
        ChatMessage::assistant(tool_calls_envelope("tc-1")),
        ChatMessage::assistant(tool_calls_envelope("tc-2")),
    ];

    let bounded = agent.bound_cached_transcript_messages(messages);
    let any_envelope = bounded.iter().any(super::assistant_message_has_tool_calls);
    assert!(
        !any_envelope,
        "all trailing tool_calls envelopes must be stripped"
    );
}

#[test]
fn integration_announcement_fires_once_for_new_toolkit() {
    // Seed the announced set with the startup-connected toolkit, mirroring the
    // turn-1 seed in `run_turn`.
    let mut announced: HashSet<String> = HashSet::new();
    announced.insert("gmail".to_string());

    // A mid-session connect adds `slack`: it should be announced, and recorded
    // so it never re-announces.
    let connected = vec!["gmail".to_string(), "slack".to_string()];
    let newly = newly_connected_slugs(&connected, &mut announced);
    assert_eq!(newly, vec!["slack".to_string()]);
    let note = integration_announcement_note(&newly)
        .expect("a newly-connected toolkit must produce an announcement");
    assert!(
        note.contains("slack"),
        "announcement must name the new toolkit slug, got: {note}"
    );
    assert!(
        !note.contains("gmail"),
        "already-announced toolkit must not be re-announced, got: {note}"
    );
    assert!(
        announced.contains("slack"),
        "the new slug must be recorded as announced"
    );

    // A second refresh with the identical connected set parks nothing — every
    // slug is now in `announced`.
    let second = newly_connected_slugs(&connected, &mut announced);
    assert!(
        second.is_empty(),
        "an unchanged connected set must not re-surface a slug, got: {second:?}"
    );
    assert!(integration_announcement_note(&second).is_none());
}

#[test]
fn integration_announcement_accumulates_two_connects_in_one_note() {
    // Two mid-session connects between consecutive user turns must BOTH be
    // announced — the second must not overwrite the first (#3044 regression:
    // the old `Option<String>` field dropped the earlier note).
    let mut announced: HashSet<String> = HashSet::new();
    announced.insert("gmail".to_string());
    let mut pending: Vec<String> = Vec::new();

    // First connect: notion.
    for slug in newly_connected_slugs(&["gmail".to_string(), "notion".to_string()], &mut announced)
    {
        if !pending.contains(&slug) {
            pending.push(slug);
        }
    }
    // Second connect before the user turn: slack.
    for slug in newly_connected_slugs(
        &[
            "gmail".to_string(),
            "notion".to_string(),
            "slack".to_string(),
        ],
        &mut announced,
    ) {
        if !pending.contains(&slug) {
            pending.push(slug);
        }
    }

    let note = integration_announcement_note(&pending).expect("two connects must produce a note");
    assert!(
        note.contains("notion"),
        "first connect must survive: {note}"
    );
    assert!(
        note.contains("slack"),
        "second connect must be present: {note}"
    );
    assert!(
        !note.contains("gmail"),
        "startup slug must not re-announce: {note}"
    );
}
