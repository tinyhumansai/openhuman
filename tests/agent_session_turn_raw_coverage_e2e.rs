use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use openhuman_core::openhuman::agent::harness::definition::AgentTier;
use openhuman_core::openhuman::agent::harness::subagent_runner::run_subagent;
use openhuman_core::openhuman::agent::harness::{
    with_parent_context, AgentDefinition, DefinitionSource, ModelSpec, ParentExecutionContext,
    PromptSource, SandboxMode, SubagentRunError, SubagentRunOptions, ToolScope,
};
use openhuman_core::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use openhuman_core::openhuman::agent::memory_loader::MemoryLoader;
use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::agent::tool_policy::{
    ToolPolicy, ToolPolicyDecision, ToolPolicyRequest,
};
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::{AgentConfig, ContextConfig, MemoryConfig};
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, ConversationMessage, Provider, ProviderDelta, ToolCall,
    UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::memory_store;
use openhuman_core::openhuman::tools::traits::ToolCallOptions;
use openhuman_core::openhuman::tools::{
    PermissionLevel, Tool, ToolContent, ToolResult, ToolScope as RuntimeToolScope,
};
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::time::{timeout, Duration};

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
    model: String,
    temperature: f64,
    messages: Vec<ChatMessage>,
    tool_names: Vec<String>,
    stream_was_requested: bool,
}

#[derive(Default)]
struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_events: Vec<ProviderDelta>,
    native_tools: bool,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            ..Self::default()
        })
    }

    fn failing(message: &'static str) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from([Err(anyhow::anyhow!(message))])),
            ..Self::default()
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(
        &self,
    ) -> openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
        openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
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
    ) -> anyhow::Result<String> {
        Ok(format!("summary: {message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.requests.lock().unwrap().push(CapturedRequest {
            model: model.to_string(),
            temperature,
            messages: request.messages.to_vec(),
            tool_names: request
                .tools
                .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
                .unwrap_or_default(),
            stream_was_requested: request.stream.is_some(),
        });
        if let Some(stream) = request.stream {
            for event in &self.stream_events {
                stream.send(event.clone()).await.ok();
            }
        }
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("default scripted final")))
    }
}

struct StaticMemory {
    entries: Mutex<Vec<MemoryEntry>>,
    fail_recall: bool,
}

impl Default for StaticMemory {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            fail_recall: false,
        }
    }
}

#[async_trait]
impl Memory for StaticMemory {
    fn name(&self) -> &str {
        "round17-memory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let id = format!("{namespace}:{key}:{}", entries.len());
        entries.push(MemoryEntry {
            id,
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-05-29T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: Some(0.95),
        });
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        if self.fail_recall {
            anyhow::bail!("forced recall failure for {query}");
        }
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.namespace.as_deref() == Some(namespace) && entry.key == key)
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| namespace.is_none_or(|ns| entry.namespace.as_deref() == Some(ns)))
            .filter(|entry| category.is_none_or(|cat| &entry.category == cat))
            .filter(|entry| session_id.is_none_or(|sid| entry.session_id.as_deref() == Some(sid)))
            .cloned()
            .collect())
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|entry| entry.namespace.as_deref() != Some(namespace) || entry.key != key);
        Ok(entries.len() != before)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(vec![NamespaceSummary {
            namespace: "round17".to_string(),
            count: self.entries.lock().unwrap().len(),
            last_updated: Some("2026-05-29T00:00:00Z".to_string()),
        }])
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.lock().unwrap().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct StaticMemoryLoader {
    context: String,
    fail: bool,
}

#[async_trait]
impl MemoryLoader for StaticMemoryLoader {
    async fn load_context(
        &self,
        _memory: &dyn Memory,
        _user_message: &str,
    ) -> anyhow::Result<String> {
        if self.fail {
            anyhow::bail!("forced loader failure");
        }
        Ok(self.context.clone())
    }
}

struct RecordingHook {
    calls: Arc<AsyncMutex<Vec<TurnContext>>>,
    notify: Arc<Notify>,
    fail: bool,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "round17-recording"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        self.calls.lock().await.push(ctx.clone());
        self.notify.notify_waiters();
        if self.fail {
            anyhow::bail!("hook failure is non-blocking");
        }
        Ok(())
    }
}

struct Round17Tool {
    name: &'static str,
    output: &'static str,
    calls: Arc<AtomicUsize>,
    error_result: bool,
    execution_error: bool,
    permission: PermissionLevel,
    scope: RuntimeToolScope,
}

impl Round17Tool {
    fn boxed(name: &'static str, output: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            calls,
            error_result: false,
            execution_error: false,
            permission: PermissionLevel::ReadOnly,
            scope: RuntimeToolScope::All,
        })
    }

    fn write(name: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output: "write-output",
            calls,
            error_result: false,
            execution_error: false,
            permission: PermissionLevel::Write,
            scope: RuntimeToolScope::All,
        })
    }

    fn failing_execute(name: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output: "not used",
            calls,
            error_result: false,
            execution_error: true,
            permission: PermissionLevel::ReadOnly,
            scope: RuntimeToolScope::All,
        })
    }

    fn tool_error(name: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output: "semantic failure",
            calls,
            error_result: true,
            execution_error: false,
            permission: PermissionLevel::ReadOnly,
            scope: RuntimeToolScope::All,
        })
    }

    fn cli_only(name: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output: "cli-only",
            calls,
            error_result: false,
            execution_error: false,
            permission: PermissionLevel::ReadOnly,
            scope: RuntimeToolScope::CliRpcOnly,
        })
    }
}

#[async_trait]
impl Tool for Round17Tool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round17 deterministic tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.execution_error {
            anyhow::bail!("execution exploded");
        }
        let suffix = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let text = if suffix.is_empty() {
            self.output.to_string()
        } else {
            format!("{}:{suffix}", self.output)
        };
        Ok(ToolResult {
            content: vec![ToolContent::Text { text: text.clone() }],
            is_error: self.error_result,
            markdown_formatted: options.prefer_markdown.then(|| format!("**{text}**")),
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn scope(&self) -> RuntimeToolScope {
        self.scope
    }
}

struct DenyNamedPolicy(&'static str);

#[async_trait]
impl ToolPolicy for DenyNamedPolicy {
    fn name(&self) -> &str {
        "round17-deny"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        if request.tool_name == self.0 {
            ToolPolicyDecision::deny("round17 policy says no")
        } else {
            ToolPolicyDecision::Allow
        }
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            input_tokens: 17,
            output_tokens: 9,
            context_window: 16_000,
            cached_input_tokens: 4,
            charged_amount_usd: 0.0003,
        }),
        reasoning_content: None,
    }
}

fn xml_tool_response(name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "pre-tool <tool_call>{{\"name\":\"{name}\",\"arguments\":{args}}}</tool_call>"
        )),
        tool_calls: vec![],
        usage: None,
        reasoning_content: Some("tool reasoning".to_string()),
    }
}

fn native_tool_response(id: &str, name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some("native preamble".to_string()),
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: Some(UsageInfo {
            input_tokens: 21,
            output_tokens: 6,
            context_window: 16_000,
            cached_input_tokens: 5,
            charged_amount_usd: 0.0004,
        }),
        reasoning_content: Some("native reasoning".to_string()),
    }
}

fn workspace(label: &str) -> (TempDir, PathBuf) {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!(
            "agent-session-turn-round17-{label}-{}",
            uuid::Uuid::new_v4()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let temp = TempDir::new_in(root.parent().unwrap()).unwrap();
    let path = temp.path().join(label);
    std::fs::create_dir_all(&path).unwrap();
    (temp, path)
}

fn memory_for_workspace(path: &PathBuf) -> Arc<dyn Memory> {
    let cfg = MemoryConfig {
        backend: "none".to_string(),
        ..MemoryConfig::default()
    };
    Arc::from(memory_store::create_memory(&cfg, path).unwrap())
}

fn agent_with(
    provider: Arc<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    workspace_path: PathBuf,
    dispatcher: Box<dyn openhuman_core::openhuman::agent::dispatcher::ToolDispatcher>,
    config: AgentConfig,
    context_config: ContextConfig,
) -> Agent {
    Agent::builder()
        .provider_arc(provider)
        .tools(tools)
        .memory(memory_for_workspace(&workspace_path))
        .memory_loader(Box::new(StaticMemoryLoader {
            context: String::new(),
            fail: false,
        }))
        .tool_dispatcher(dispatcher)
        .workspace_dir(workspace_path)
        .event_context("round17-session", "round17-channel")
        .agent_definition_name("round17/orchestrator")
        .config(config)
        .context_config(context_config)
        .auto_save(true)
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap()
}

#[tokio::test]
async fn turn_native_tool_progress_reasoning_usage_and_resume_seed_paths() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("native-progress");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(native_tool_response(
                    "native-1",
                    "round17_echo",
                    json!({ "value": "alpha" }),
                )),
                Ok(ChatResponse {
                    text: Some("native final".to_string()),
                    tool_calls: vec![],
                    usage: Some(UsageInfo {
                        input_tokens: 5,
                        output_tokens: 3,
                        context_window: 16_000,
                        cached_input_tokens: 2,
                        charged_amount_usd: 0.0001,
                    }),
                    reasoning_content: Some("final hidden reasoning".to_string()),
                }),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        stream_events: vec![
            ProviderDelta::TextDelta {
                delta: "stream text".to_string(),
            },
            ProviderDelta::ThinkingDelta {
                delta: "stream thought".to_string(),
            },
            ProviderDelta::ToolCallStart {
                call_id: "native-1".to_string(),
                tool_name: "round17_echo".to_string(),
            },
            ProviderDelta::ToolCallArgsDelta {
                call_id: "native-1".to_string(),
                delta: "{\"value\":\"alpha\"}".to_string(),
            },
        ],
        native_tools: true,
    });
    let mut agent = agent_with(
        provider.clone(),
        vec![Round17Tool::boxed(
            "round17_echo",
            "echo-output",
            calls.clone(),
        )],
        workspace_path,
        Box::new(NativeToolDispatcher),
        AgentConfig {
            max_tool_iterations: 4,
            max_history_messages: 12,
            ..AgentConfig::default()
        },
        ContextConfig {
            prefer_markdown_tool_output: true,
            ..ContextConfig::default()
        },
    );
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));

    let answer = agent.turn("use the native tool").await.unwrap();
    assert_eq!(answer, "native final");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(agent.history().iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls {
            tool_calls,
            reasoning_content,
            ..
        } if tool_calls[0].id == "native-1" && reasoning_content.as_deref() == Some("native reasoning")
    )));
    assert!(agent.history().iter().any(|message| matches!(
        message,
        ConversationMessage::Chat(chat)
            if chat.role == "assistant"
                && chat.extra_metadata.as_ref()
                    .and_then(|m| m.get("reasoning_content"))
                    .and_then(|v| v.as_str()) == Some("final hidden reasoning")
    )));

    let mut progress = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        progress.push(event);
    }
    assert!(progress
        .iter()
        .any(|event| matches!(event, AgentProgress::TurnStarted)));
    assert!(progress.iter().any(|event| matches!(
        event,
        AgentProgress::TextDelta { delta, iteration: 1 } if delta == "stream text"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        AgentProgress::ThinkingDelta { delta, iteration: 1 } if delta == "stream thought"
    )));
    assert!(progress.iter().any(|event| matches!(
        event,
        AgentProgress::ToolCallCompleted { tool_name, success, .. }
            if tool_name == "round17_echo" && *success
    )));

    let requests = provider.requests();
    assert!(requests[0].stream_was_requested);
    assert_eq!(requests[0].tool_names, vec!["round17_echo"]);
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.role == "tool"
                && message.content.contains("**echo-output:alpha**"))
    );

    let (_seeded_tmp, seeded_workspace) = workspace("seeded-resume");
    let mut seeded = agent_with(
        ScriptedProvider::new(vec![text_response("seeded final")]),
        vec![Round17Tool::boxed(
            "round17_echo",
            "unused",
            Arc::new(AtomicUsize::new(0)),
        )],
        seeded_workspace,
        Box::new(XmlToolDispatcher),
        AgentConfig {
            max_history_messages: 3,
            ..AgentConfig::default()
        },
        ContextConfig::default(),
    );
    seeded
        .seed_resume_from_messages(
            vec![
                ("user".to_string(), "old one".to_string()),
                ("assistant".to_string(), "old two".to_string()),
                ("user".to_string(), "current".to_string()),
            ],
            "current",
        )
        .unwrap();
    let seeded_answer = seeded.run_single("current").await.unwrap();
    assert_eq!(seeded_answer, "seeded final");
}

#[tokio::test]
async fn turn_xml_failures_checkpoint_policy_visibility_and_hooks_are_publicly_exercised() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("xml-failures");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let ok_calls = Arc::new(AtomicUsize::new(0));
    let err_calls = Arc::new(AtomicUsize::new(0));
    let boom_calls = Arc::new(AtomicUsize::new(0));
    let write_calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(xml_tool_response("hidden_tool", json!({ "value": "h" }))),
                Ok(xml_tool_response("cli_only", json!({ "value": "c" }))),
                Ok(xml_tool_response("round17_error", json!({ "value": "e" }))),
                Ok(xml_tool_response("round17_boom", json!({ "value": "b" }))),
                Ok(xml_tool_response("round17_write", json!({ "value": "w" }))),
                Ok(xml_tool_response("round17_ok", json!({ "value": "o" }))),
                Ok(ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                }),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        ..ScriptedProvider::default()
    });
    let hook_calls = Arc::new(AsyncMutex::new(Vec::<TurnContext>::new()));
    let hook_notify = Arc::new(Notify::new());
    let mut channel_permissions = std::collections::HashMap::new();
    channel_permissions.insert("round17-channel".to_string(), "read_only".to_string());
    let mut agent = Agent::builder()
        .provider_arc(provider.clone())
        .tools(vec![
            Round17Tool::boxed("round17_ok", "ok-output", ok_calls.clone()),
            Round17Tool::tool_error("round17_error", err_calls.clone()),
            Round17Tool::failing_execute("round17_boom", boom_calls.clone()),
            Round17Tool::write("round17_write", write_calls.clone()),
            Round17Tool::cli_only("cli_only", Arc::new(AtomicUsize::new(0))),
        ])
        .memory(Arc::new(StaticMemory {
            entries: Mutex::new(vec![MemoryEntry {
                id: "m1".to_string(),
                key: "k1".to_string(),
                content: "remembered citation".to_string(),
                namespace: Some("round17".to_string()),
                category: MemoryCategory::Conversation,
                timestamp: "2026-05-29T00:00:00Z".to_string(),
                session_id: None,
                score: Some(0.9),
            }]),
            fail_recall: true,
        }))
        .memory_loader(Box::new(StaticMemoryLoader {
            context: "[round17 injected context]\n".to_string(),
            fail: true,
        }))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("round17-session", "round17-channel")
        .agent_definition_name("round17/orchestrator")
        .config(AgentConfig {
            max_tool_iterations: 6,
            channel_permissions,
            ..AgentConfig::default()
        })
        .context_config(ContextConfig {
            tool_result_budget_bytes: 96,
            ..ContextConfig::default()
        })
        .post_turn_hooks(vec![Arc::new(RecordingHook {
            calls: hook_calls.clone(),
            notify: hook_notify.clone(),
            fail: true,
        })])
        .tool_policy(Arc::new(DenyNamedPolicy("round17_ok")))
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap();
    let mut visible = HashSet::new();
    visible.insert("round17_ok".to_string());
    visible.insert("round17_error".to_string());
    visible.insert("round17_boom".to_string());
    visible.insert("round17_write".to_string());
    visible.insert("cli_only".to_string());
    agent.set_visible_tool_names(visible);

    let checkpoint = agent.turn("exercise failure branches").await.unwrap();
    assert!(
        checkpoint.contains("Done so far") || checkpoint.contains("Need next"),
        "fallback checkpoint should be deterministic, got {checkpoint}"
    );
    assert_eq!(ok_calls.load(Ordering::SeqCst), 0);
    assert_eq!(err_calls.load(Ordering::SeqCst), 1);
    assert_eq!(boom_calls.load(Ordering::SeqCst), 1);
    assert_eq!(write_calls.load(Ordering::SeqCst), 0);
    assert!(agent.take_last_turn_citations().is_empty());

    timeout(Duration::from_secs(1), async {
        loop {
            if !hook_calls.lock().await.is_empty() {
                break;
            }
            hook_notify.notified().await;
        }
    })
    .await
    .unwrap();
    let hooks = hook_calls.lock().await;
    assert_eq!(hooks[0].assistant_response, checkpoint);
    assert_eq!(hooks[0].tool_calls.len(), 6);

    let joined = provider
        .requests()
        .into_iter()
        .flat_map(|request| request.messages)
        .map(|message| message.content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("not available to this agent"));
    assert!(joined.contains("semantic failure"));
    assert!(joined.contains("Error executing round17_boom"));
    assert!(joined.contains("denied by policy 'round17-deny'"));

    let (_failing_tmp, failing_workspace) = workspace("provider-error");
    let provider_error = ScriptedProvider::failing("provider offline");
    let mut failing_agent = agent_with(
        provider_error,
        vec![],
        failing_workspace,
        Box::new(XmlToolDispatcher),
        AgentConfig::default(),
        ContextConfig::default(),
    );
    let err = failing_agent.run_single("fail now").await.unwrap_err();
    assert!(err.to_string().contains("provider offline"));
}

#[tokio::test]
async fn subagent_runner_parent_context_filters_tools_caps_output_and_reports_errors() {
    let _env = env_lock();
    let no_parent = run_subagent(
        &definition("round17_child", ToolScope::Wildcard, None, 3),
        "outside turn",
        SubagentRunOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(no_parent, SubagentRunError::NoParentContext));

    let (_temp, workspace_path) = workspace("subagent");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let echo_calls = Arc::new(AtomicUsize::new(0));
    let hidden_calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(native_tool_response(
                    "child-1",
                    "round17_echo",
                    json!({ "value": "child" }),
                )),
                Ok(text_response("child final response that should be capped")),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        native_tools: true,
        ..ScriptedProvider::default()
    });
    let all_tools = vec![
        Round17Tool::boxed("round17_echo", "child-tool", echo_calls.clone()),
        Round17Tool::boxed("round17_hidden", "hidden-tool", hidden_calls.clone()),
        Round17Tool::boxed(
            "spawn_subagent",
            "must-strip",
            Arc::new(AtomicUsize::new(0)),
        ),
    ];
    let all_specs = all_tools.iter().map(|tool| tool.spec()).collect::<Vec<_>>();
    let parent = ParentExecutionContext {
        provider: provider.clone(),
        all_tools: Arc::new(all_tools),
        all_tool_specs: Arc::new(all_specs),
        model_name: "parent-model".to_string(),
        temperature: 0.22,
        workspace_dir: workspace_path.clone(),
        memory: Arc::new(StaticMemory::default()),
        agent_config: AgentConfig {
            max_tool_iterations: 5,
            ..AgentConfig::default()
        },
        skills: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory context".to_string())),
        session_id: "round17-parent-session".to_string(),
        channel: "round17-parent-channel".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: openhuman_core::openhuman::context::prompt::ToolCallFormat::Json,
        session_key: "123_parent".to_string(),
        session_parent_prefix: Some("root_ancestor".to_string()),
        on_progress: None,
    };

    let outcome = with_parent_context(parent.clone(), async {
        run_subagent(
            &definition(
                "round17_child",
                ToolScope::Named(vec![
                    "round17_echo".to_string(),
                    "round17_hidden".to_string(),
                    "spawn_subagent".to_string(),
                ]),
                Some(18),
                4,
            ),
            "delegate this",
            SubagentRunOptions {
                context: Some("spawn context".to_string()),
                model_override: Some("override-model".to_string()),
                task_id: Some("task-round17".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await
    })
    .await
    .unwrap();

    assert_eq!(outcome.task_id, "task-round17");
    assert_eq!(outcome.agent_id, "round17_child");
    assert_eq!(outcome.iterations, 2);
    assert!(outcome.output.ends_with("[...truncated]"));
    assert_eq!(echo_calls.load(Ordering::SeqCst), 1);
    assert_eq!(hidden_calls.load(Ordering::SeqCst), 0);

    let requests = provider.requests();
    assert_eq!(requests[0].model, "override-model");
    assert_eq!(requests[0].temperature, 0.4);
    assert_eq!(requests[0].tool_names, vec!["round17_echo"]);
    assert!(requests[0]
        .messages
        .iter()
        .any(|message| message.role == "system"
            && message.content.contains("Sub-agent Role Contract")
            && message.content.contains("round17 child prompt")));
    assert!(requests[0]
        .messages
        .iter()
        .any(|message| message.role == "user"
            && message.content.contains("spawn context")
            && message.content.contains("delegate this")));

    let error_parent = ParentExecutionContext {
        provider: ScriptedProvider::failing("subagent provider offline"),
        ..parent
    };
    let provider_err = with_parent_context(error_parent, async {
        run_subagent(
            &definition("round17_provider_error", ToolScope::Wildcard, None, 1),
            "provider error",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap_err();
    assert!(matches!(provider_err, SubagentRunError::Provider(_)));
    assert!(provider_err
        .to_string()
        .contains("subagent provider offline"));
}

fn definition(
    id: &str,
    tools: ToolScope,
    max_result_chars: Option<usize>,
    max_iterations: usize,
) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        when_to_use: "round17 test definition".to_string(),
        display_name: Some(id.to_string()),
        system_prompt: PromptSource::Inline("round17 child prompt".to_string()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools,
        disallowed_tools: vec!["round17_hidden".to_string()],
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations,
        iteration_policy: Default::default(),
        max_result_chars,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::Builtin,
    }
}
