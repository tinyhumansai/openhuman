use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::{NativeToolDispatcher, XmlToolDispatcher};
use openhuman_core::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::config::{AgentConfig, ContextConfig};
use openhuman_core::openhuman::context::session_memory::SessionMemoryConfig;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolContent, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{sleep, Duration, Instant};

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
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    native_tools: bool,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>, native_tools: bool) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
            native_tools,
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
        Ok(format!("checkpoint: {message}"))
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
            .unwrap_or_else(|| Ok(text_response("fallback final", None)))
    }
}

struct RecordingMemory {
    stores: Mutex<Vec<(String, String, String, MemoryCategory)>>,
}

impl RecordingMemory {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stores: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Memory for RecordingMemory {
    fn name(&self) -> &str {
        "round20-recording-memory"
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
            id: "round20-memory".to_string(),
            key: "preference".to_string(),
            content: "Use concise progress updates.".to_string(),
            namespace: Some("user_profile".to_string()),
            category: MemoryCategory::Core,
            timestamp: "2026-05-30T00:00:00Z".to_string(),
            session_id: None,
            score: Some(0.91),
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
        Ok(vec![NamespaceSummary {
            namespace: "user_profile".to_string(),
            count: 1,
            last_updated: Some("2026-05-30T00:00:00Z".to_string()),
        }])
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.stores.lock().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct Round20Tool {
    name: &'static str,
    output: &'static str,
    calls: Arc<AtomicUsize>,
    seen_args: Arc<Mutex<Vec<serde_json::Value>>>,
    is_error: bool,
}

struct RecordingHook {
    calls: Arc<AtomicUsize>,
    contexts: Arc<Mutex<Vec<TurnContext>>>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "round20-recording-hook"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> Result<()> {
        self.contexts.lock().push(ctx.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl Tool for Round20Tool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round20 deterministic tool"
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
        self.seen_args.lock().push(args.clone());
        let value = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("empty");
        Ok(ToolResult {
            content: vec![ToolContent::Text {
                text: format!("{}:{value}", self.output),
            }],
            is_error: self.is_error,
            markdown_formatted: None,
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
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

fn native_tool_response(name: &str, arguments: &str) -> ChatResponse {
    ChatResponse {
        text: Some("native call".to_string()),
        tool_calls: vec![ToolCall {
            id: "round20-native-1".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            extra_content: None,
        }],
        usage: Some(UsageInfo {
            input_tokens: 7_000,
            output_tokens: 600,
            context_window: 16_000,
            cached_input_tokens: 250,
            charged_amount_usd: 0.002,
        }),
        reasoning_content: Some("native hidden reasoning".to_string()),
    }
}

fn xml_tool_response(name: &str, value: &str) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "before <tool_call>{{\"name\":\"{name}\",\"arguments\":{{\"value\":\"{value}\"}}}}</tool_call>"
        )),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 5_000,
            output_tokens: 500,
            context_window: 16_000,
            cached_input_tokens: 100,
            charged_amount_usd: 0.001,
        }),
        reasoning_content: None,
    }
}

fn workspace(label: &str) -> (TempDir, PathBuf) {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!(
            "agent-turn-builder-leftovers-round20-{label}-{}",
            uuid::Uuid::new_v4()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let temp = TempDir::new_in(root.parent().unwrap()).unwrap();
    let path = temp.path().join(label);
    std::fs::create_dir_all(&path).unwrap();
    (temp, path)
}

fn tool(
    name: &'static str,
    output: &'static str,
    calls: Arc<AtomicUsize>,
    seen_args: Arc<Mutex<Vec<serde_json::Value>>>,
    is_error: bool,
) -> Box<dyn Tool> {
    Box::new(Round20Tool {
        name,
        output,
        calls,
        seen_args,
        is_error,
    })
}

#[tokio::test]
async fn native_turn_dedups_duplicate_tool_specs_and_recovers_invalid_arguments() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("native-dedup-invalid-args");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);

    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let seen_args = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider::new(
        vec![
            native_tool_response("round20_dup", "{not valid json"),
            text_response("native final", None),
        ],
        true,
    );

    let mut agent = Agent::builder()
        .provider_arc(provider.clone())
        .tools(vec![
            tool(
                "round20_dup",
                "first-tool",
                first_calls.clone(),
                seen_args.clone(),
                false,
            ),
            tool(
                "round20_dup",
                "second-tool",
                second_calls.clone(),
                Arc::new(Mutex::new(Vec::new())),
                false,
            ),
        ])
        .visible_tool_names(["round20_dup".to_string()].into_iter().collect())
        .memory(RecordingMemory::new())
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("round20-native-session", "round20-native-channel")
        .agent_definition_name("round20/native")
        .context_config(ContextConfig {
            prefer_markdown_tool_output: false,
            ..ContextConfig::default()
        })
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap();

    let answer = agent.turn("call the duplicate native tool").await.unwrap();

    assert_eq!(answer, "native final");
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 0);
    assert_eq!(seen_args.lock().as_slice(), &[json!({})]);
    assert_eq!(provider.requests()[0].tool_names, vec!["round20_dup"]);
    assert!(provider.requests()[1]
        .messages
        .iter()
        .any(|message| message.role == "tool" && message.content.contains("first-tool:empty")));
}

#[tokio::test]
async fn xml_turn_persists_tool_cycle_and_fires_failure_hook_context() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("xml-hook-persistence");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_contexts = Arc::new(Mutex::new(Vec::new()));
    let failure_calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new(
        vec![
            xml_tool_response("round20_fail", "bad"),
            text_response("xml final", None),
        ],
        false,
    );

    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(vec![tool(
            "round20_fail",
            "semantic failure",
            failure_calls.clone(),
            Arc::new(Mutex::new(Vec::new())),
            true,
        )])
        .memory(RecordingMemory::new())
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path.clone())
        .event_context("round20-hook-session", "round20-hook-channel")
        .agent_definition_name("round20/xml")
        .post_turn_hooks(vec![Arc::new(RecordingHook {
            calls: hook_calls.clone(),
            contexts: hook_contexts.clone(),
        })])
        .config(AgentConfig {
            max_tool_iterations: 3,
            ..AgentConfig::default()
        })
        .context_config(ContextConfig {
            prefer_markdown_tool_output: false,
            ..ContextConfig::default()
        })
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(true)
        .build()
        .unwrap();

    let answer = agent
        .run_single("record a failing tool outcome")
        .await
        .unwrap();
    assert_eq!(answer, "xml final");
    assert_eq!(failure_calls.load(Ordering::SeqCst), 1);

    wait_for_hook_calls(&hook_calls, 1).await;
    let contexts = hook_contexts.lock();
    assert_eq!(contexts[0].assistant_response, "xml final");
    assert_eq!(contexts[0].tool_calls.len(), 1);
    assert_eq!(contexts[0].tool_calls[0].name, "round20_fail");
    assert!(!contexts[0].tool_calls[0].success);
    assert!(contexts[0].tool_calls[0].output_summary.contains("failed"));

    let raw_transcripts = workspace_path.join("session_raw");
    let transcript = find_jsonl(&raw_transcripts).expect("raw transcript should be persisted");
    let transcript_body = std::fs::read_to_string(transcript).unwrap();
    assert!(transcript_body.contains("round20_fail"));
    assert!(transcript_body.contains("semantic failure:bad"));
    assert!(transcript_body.contains("xml final"));
}

#[tokio::test]
async fn session_memory_threshold_path_runs_only_after_successful_turn() {
    let _env = env_lock();
    let (_temp, workspace_path) = workspace("session-memory-threshold");
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_contexts = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ScriptedProvider::new(
        vec![
            xml_tool_response("round20_ok", "flush"),
            text_response(
                "flush final",
                Some(UsageInfo {
                    input_tokens: 8_000,
                    output_tokens: 1_000,
                    context_window: 16_000,
                    cached_input_tokens: 10,
                    charged_amount_usd: 0.003,
                }),
            ),
        ],
        false,
    );

    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(vec![tool(
            "round20_ok",
            "ok-output",
            calls.clone(),
            Arc::new(Mutex::new(Vec::new())),
            false,
        )])
        .memory(RecordingMemory::new())
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("round20-flush-session", "round20-flush-channel")
        .agent_definition_name("round20/flush")
        .post_turn_hooks(vec![Arc::new(RecordingHook {
            calls: hook_calls.clone(),
            contexts: hook_contexts.clone(),
        })])
        .config(AgentConfig {
            max_tool_iterations: 2,
            ..AgentConfig::default()
        })
        .context_config(ContextConfig {
            session_memory: SessionMemoryConfig {
                min_token_growth: 1,
                min_tool_calls: 1,
                min_turns_between: 1,
            },
            prefer_markdown_tool_output: false,
            ..ContextConfig::default()
        })
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(true)
        .build()
        .unwrap();

    let answer = agent
        .turn("trigger session memory thresholds")
        .await
        .unwrap();
    assert_eq!(answer, "flush final");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    wait_for_hook_calls(&hook_calls, 1).await;
    assert_eq!(hook_contexts.lock()[0].iteration_count, 2);

    let (_empty_tmp, empty_workspace) = workspace("empty-failed-turn");
    let empty_provider = ScriptedProvider::new(vec![text_response("   ", None)], false);
    let mut failed_agent = Agent::builder()
        .provider_arc(empty_provider)
        .tools(Vec::new())
        .memory(RecordingMemory::new())
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(empty_workspace)
        .event_context("round20-empty-session", "round20-empty-channel")
        .agent_definition_name("round20/empty")
        .explicit_preferences_enabled(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap();
    let err = failed_agent.run_single("return blank").await.unwrap_err();
    assert!(
        err.to_string().contains("empty response"),
        "expected empty-response error, got {err}"
    );
    assert!(failed_agent.history().iter().all(|message| {
        !serde_json::to_string(message)
            .unwrap_or_default()
            .contains("assistant_resp")
    }));
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

fn find_jsonl(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = std::fs::read_dir(path).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                return Some(path);
            }
        }
    }
    None
}
