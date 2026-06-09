use async_trait::async_trait;
use openhuman_core::core::event_bus::{init_global, request_native_global, DEFAULT_CAPACITY};
use openhuman_core::openhuman::agent::bus::{
    register_agent_handlers, AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD,
};
use openhuman_core::openhuman::agent::debug::{dump_agent_prompt, DumpPromptOptions};
use openhuman_core::openhuman::agent::dispatcher::XmlToolDispatcher;
use openhuman_core::openhuman::agent::{Agent, AgentBuilder};
use openhuman_core::openhuman::config::{AgentConfig, MultimodalConfig, MultimodalFileConfig};
use openhuman_core::openhuman::context::prompt::LearnedContextData;
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderDelta, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolContent, ToolResult, ToolScope};
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct CapturedTurn {
    messages: Vec<ChatMessage>,
    tool_names: Vec<String>,
}

#[derive(Default)]
struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    turns: Mutex<Vec<CapturedTurn>>,
    native_tools: bool,
    vision: bool,
    stream_events: Vec<ProviderDelta>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            ..Self::default()
        })
    }

    fn failing(message: &str) -> Arc<Self> {
        let mut responses = VecDeque::new();
        responses.push_back(Err(anyhow::anyhow!(message.to_string())));
        Arc::new(Self {
            responses: Mutex::new(responses),
            ..Self::default()
        })
    }

    fn turns(&self) -> Vec<CapturedTurn> {
        self.turns.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: self.native_tools,
            vision: self.vision,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(message.to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.turns.lock().unwrap().push(CapturedTurn {
            messages: request.messages.to_vec(),
            tool_names: request
                .tools
                .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
                .unwrap_or_default(),
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
            .unwrap_or_else(|| Ok(ChatResponse::default()))
    }
}

struct StaticTool {
    name: &'static str,
    output: &'static str,
    is_error: bool,
    scope: ToolScope,
    permission: PermissionLevel,
    cap: Option<usize>,
}

impl StaticTool {
    fn ok(name: &'static str, output: &'static str) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            is_error: false,
            scope: ToolScope::All,
            permission: PermissionLevel::ReadOnly,
            cap: None,
        })
    }

    fn err(name: &'static str, output: &'static str) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            is_error: true,
            scope: ToolScope::All,
            permission: PermissionLevel::ReadOnly,
            cap: None,
        })
    }

    fn cli_only(name: &'static str) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output: "cli-only-output",
            is_error: false,
            scope: ToolScope::CliRpcOnly,
            permission: PermissionLevel::ReadOnly,
            cap: None,
        })
    }

    fn capped(name: &'static str, output: &'static str, cap: usize) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            is_error: false,
            scope: ToolScope::All,
            permission: PermissionLevel::ReadOnly,
            cap: Some(cap),
        })
    }
}

#[async_trait]
impl Tool for StaticTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round15 deterministic test tool"
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
        let suffix = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let body = if suffix.is_empty() {
            self.output.to_string()
        } else {
            format!("{}:{suffix}", self.output)
        };
        Ok(ToolResult {
            content: vec![ToolContent::Text { text: body }],
            is_error: self.is_error,
            markdown_formatted: None,
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn scope(&self) -> ToolScope {
        self.scope
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        self.cap
    }
}

#[derive(Default)]
struct NoopMemory {
    entries: Mutex<Vec<MemoryEntry>>,
}

#[async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "round15-noop"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.entries.lock().unwrap().push(MemoryEntry {
            id: format!("{namespace}:{key}"),
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-05-29T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: Some(1.0),
            taint: Default::default(),
        });
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
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
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| namespace.is_none_or(|ns| entry.namespace.as_deref() == Some(ns)))
            .filter(|entry| category.is_none_or(|cat| &entry.category == cat))
            .cloned()
            .collect())
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries
            .retain(|entry| !(entry.namespace.as_deref() == Some(namespace) && entry.key == key));
        Ok(entries.len() != before)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(vec![NamespaceSummary {
            namespace: "round15".to_string(),
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

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            input_tokens: 11,
            output_tokens: 7,
            context_window: 16_000,
            cached_input_tokens: 3,
            charged_amount_usd: 0.0001,
        }),
        reasoning_content: None,
    }
}

fn native_tool_response(name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some("using native tool".to_string()),
        tool_calls: vec![ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments: arguments.to_string(),
            extra_content: None,
        }],
        usage: Some(UsageInfo {
            input_tokens: 13,
            output_tokens: 5,
            context_window: 16_000,
            cached_input_tokens: 2,
            charged_amount_usd: 0.0002,
        }),
        reasoning_content: Some("private scratchpad".to_string()),
    }
}

fn xml_tool_response(name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "prelude <tool_call>{{\"name\":\"{name}\",\"arguments\":{arguments}}}</tool_call>"
        )),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

async fn run_bus_turn(
    provider: Arc<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    max_tool_iterations: usize,
    visible_tool_names: Option<HashSet<String>>,
) -> Result<AgentTurnResponse, String> {
    init_global(DEFAULT_CAPACITY);
    register_agent_handlers();
    request_native_global::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        AgentTurnRequest {
            provider,
            history: vec![ChatMessage::system("system"), ChatMessage::user("run")],
            tools_registry: Arc::new(tools),
            provider_name: "round15".to_string(),
            model: "gpt-4o-mini".to_string(),
            temperature: 0.0,
            silent: true,
            channel_name: "round15".to_string(),
            multimodal: MultimodalConfig::default(),
            multimodal_files: MultimodalFileConfig::default(),
            max_tool_iterations,
            on_delta: None,
            target_agent_id: Some("orchestrator".to_string()),
            visible_tool_names,
            extra_tools: Vec::new(),
            on_progress: None,
            origin: openhuman_core::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
        },
    )
    .await
    .map_err(|err| err.to_string())
}

#[tokio::test]
async fn bus_turn_native_tools_dedups_streams_and_records_tool_messages() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(native_tool_response("echo", json!({ "value": "alpha" }))),
                Ok(text_response("final native answer")),
            ]
            .into(),
        ),
        turns: Mutex::new(Vec::new()),
        native_tools: true,
        vision: false,
        stream_events: vec![
            ProviderDelta::TextDelta {
                delta: "draft ".to_string(),
            },
            ProviderDelta::ThinkingDelta {
                delta: "thinking".to_string(),
            },
            ProviderDelta::ToolCallStart {
                call_id: "call-echo".to_string(),
                tool_name: "echo".to_string(),
            },
            ProviderDelta::ToolCallArgsDelta {
                call_id: "call-echo".to_string(),
                delta: "{\"value\"".to_string(),
            },
        ],
    });
    let response = run_bus_turn(
        provider.clone(),
        vec![
            StaticTool::ok("echo", "first"),
            StaticTool::ok("echo", "duplicate"),
            StaticTool::ok("other", "unused"),
        ],
        4,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.text, "final native answer");
    let turns = provider.turns();
    assert_eq!(turns[0].tool_names, vec!["echo", "other"]);
    assert!(
        turns[1]
            .messages
            .iter()
            .any(|msg| msg.role == "tool" && msg.content.contains("first:alpha")),
        "second native request should carry a role=tool result message"
    );
}

#[tokio::test]
async fn bus_turn_prompt_mode_covers_invisible_cli_only_and_unknown_tools() {
    let mut visible = HashSet::new();
    visible.insert("allowed".to_string());
    let invisible_provider = ScriptedProvider::new(vec![
        xml_tool_response("hidden", json!({ "value": "x" })),
        text_response("after invisible"),
    ]);
    let invisible_response = run_bus_turn(
        invisible_provider.clone(),
        vec![StaticTool::ok("allowed", "allowed")],
        4,
        Some(visible),
    )
    .await
    .unwrap();
    assert_eq!(invisible_response.text, "after invisible");

    let provider = ScriptedProvider::new(vec![
        xml_tool_response("cli_only", json!({ "value": "x" })),
        xml_tool_response("missing", json!({ "value": "x" })),
        text_response("recovered"),
    ]);

    let response = run_bus_turn(
        provider.clone(),
        vec![
            StaticTool::ok("allowed", "allowed"),
            StaticTool::cli_only("cli_only"),
        ],
        6,
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.text, "recovered");
    let joined = provider
        .turns()
        .into_iter()
        .flat_map(|turn| turn.messages)
        .map(|msg| msg.content)
        .collect::<Vec<_>>()
        .join("\n");
    let invisible_joined = invisible_provider
        .turns()
        .into_iter()
        .flat_map(|turn| turn.messages)
        .map(|msg| msg.content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(invisible_joined.contains("Unknown tool: hidden"));
    assert!(joined.contains("only available via explicit CLI/RPC invocation"));
    assert!(joined.contains("Unknown tool: missing"));
}

#[tokio::test]
async fn bus_turn_halts_on_repeated_tool_error_and_truncates_capped_result() {
    let provider = ScriptedProvider::new(vec![
        xml_tool_response("capper", json!({ "value": "" })),
        xml_tool_response("fail", json!({ "value": "same" })),
        xml_tool_response("fail", json!({ "value": "same" })),
        xml_tool_response("fail", json!({ "value": "same" })),
    ]);

    let response = run_bus_turn(
        provider.clone(),
        vec![
            StaticTool::err("fail", "boom"),
            StaticTool::capped("capper", "abcdefghijklmnopqrstuvwxyz", 5),
        ],
        8,
        None,
    )
    .await
    .unwrap();

    assert!(response.text.contains("retried 3 times"));
    assert!(response.text.contains("boom:same"));
    let joined = provider
        .turns()
        .into_iter()
        .flat_map(|turn| turn.messages)
        .map(|msg| msg.content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("[truncated by tool cap: 21 more chars not shown]"));
}

#[tokio::test]
async fn bus_turn_surfaces_provider_error_and_iteration_cap() {
    let provider_error = run_bus_turn(
        ScriptedProvider::failing("provider unavailable"),
        vec![StaticTool::ok("echo", "ok")],
        2,
        None,
    )
    .await
    .err()
    .expect("provider error should surface");
    assert!(provider_error.contains("provider unavailable"));

    let capped = run_bus_turn(
        ScriptedProvider::new(vec![xml_tool_response("missing", json!({ "value": "x" }))]),
        vec![StaticTool::ok("echo", "ok")],
        1,
        None,
    )
    .await
    .err()
    .expect("iteration cap should surface");
    assert!(capped.contains("maximum tool iterations"));
}

#[tokio::test]
async fn agent_builder_prompt_and_debug_dump_cover_public_session_paths() {
    let workspace = round15_workspace("session-prompt");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("PROFILE.md"), "Round15 profile").unwrap();
    std::fs::write(workspace.join("MEMORY.md"), "Round15 memory").unwrap();

    let provider = ScriptedProvider::new(vec![text_response("unused")]);
    let mut config = AgentConfig::default();
    config.max_tool_iterations = 2;
    config.max_history_messages = 4;

    let agent = AgentBuilder::new()
        .provider_arc(provider)
        .tools(vec![StaticTool::ok("echo", "ok")])
        .memory(Arc::new(NoopMemory::default()))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .config(config)
        .workspace_dir(workspace.clone())
        .agent_definition_name("round15/orchestrator")
        .event_context("round15-session", "round15-channel")
        .omit_profile(false)
        .omit_memory_md(false)
        .unified_compaction_enabled(false)
        .build()
        .unwrap();

    assert_eq!(agent.agent_definition_name(), "round15/orchestrator");
    assert!(agent.session_key().contains("round15_orchestrator"));
    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .unwrap();
    assert!(prompt.contains("Round15 profile"));
    assert!(prompt.contains("Round15 memory"));
    assert!(prompt.contains("echo"));

    let dump_err = dump_agent_prompt(DumpPromptOptions {
        agent_id: "integrations_agent".to_string(),
        toolkit: None,
        workspace_dir_override: Some(workspace),
        model_override: Some("round15-model".to_string()),
    })
    .await
    .unwrap_err()
    .to_string();
    assert!(dump_err.contains("integrations_agent requires a `toolkit` argument"));
}

#[tokio::test]
async fn agent_turn_blank_final_response_is_typed_error() {
    let workspace = round15_workspace("blank-final");
    std::fs::create_dir_all(&workspace).unwrap();
    let provider = ScriptedProvider::new(vec![ChatResponse::default()]);
    let mut agent = Agent::builder()
        .provider_arc(provider)
        .tools(vec![])
        .memory(Arc::new(NoopMemory::default()))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .config(AgentConfig {
            max_tool_iterations: 1,
            ..AgentConfig::default()
        })
        .workspace_dir(workspace)
        .build()
        .unwrap();

    let err = agent.turn("blank please").await.unwrap_err().to_string();
    assert!(err.contains("empty response"));
}

fn round15_workspace(label: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!(
            "agent-tool-loop-round15-{label}-{}",
            uuid::Uuid::new_v4()
        ))
}
