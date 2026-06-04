use async_trait::async_trait;
use openhuman_core::core::event_bus::{init_global, request_native_global, DEFAULT_CAPACITY};
use openhuman_core::openhuman::agent::bus::{
    register_agent_handlers, AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD,
};
use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::config::{MultimodalConfig, MultimodalFileConfig};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderDelta, UsageInfo,
};
use openhuman_core::openhuman::security::POLICY_BLOCKED_MARKER;
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolContent, ToolResult, ToolScope};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    tool_names: Vec<String>,
    streamed: bool,
}

#[derive(Default)]
struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_events: Vec<ProviderDelta>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            ..Self::default()
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
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
    ) -> anyhow::Result<String> {
        Ok(message.to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.requests.lock().unwrap().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tool_names: request
                .tools
                .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
                .unwrap_or_default(),
            streamed: request.stream.is_some(),
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
            .unwrap_or_else(|| Ok(text_response("script exhausted fallback")))
    }
}

struct Round22Tool {
    name: &'static str,
    output: &'static str,
    is_error: bool,
}

impl Round22Tool {
    fn ok(name: &'static str, output: &'static str) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            is_error: false,
        })
    }

    fn err(name: &'static str, output: &'static str) -> Box<dyn Tool> {
        Box::new(Self {
            name,
            output,
            is_error: true,
        })
    }
}

#[async_trait]
impl Tool for Round22Tool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "round22 deterministic coverage tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" },
                "command": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let suffix = args
            .get("value")
            .or_else(|| args.get("command"))
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
        PermissionLevel::ReadOnly
    }

    fn scope(&self) -> ToolScope {
        ToolScope::All
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            input_tokens: 3,
            output_tokens: 2,
            context_window: 16_000,
            cached_input_tokens: 1,
            charged_amount_usd: 0.00001,
        }),
        reasoning_content: None,
    }
}

fn xml_tool_response(name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "before <tool_call>{{\"name\":\"{name}\",\"arguments\":{args}}}</tool_call>"
        )),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

fn glm_response(line: &str) -> ChatResponse {
    ChatResponse {
        text: Some(line.to_string()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

async fn run_turn(
    provider: Arc<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
) -> Result<AgentTurnResponse, String> {
    init_global(DEFAULT_CAPACITY);
    register_agent_handlers();
    request_native_global::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        AgentTurnRequest {
            provider,
            history: vec![
                ChatMessage::system("round22 system"),
                ChatMessage::user("round22 run"),
            ],
            tools_registry: Arc::new(tools),
            provider_name: "round22".to_string(),
            model: "gpt-4o-mini".to_string(),
            temperature: 0.0,
            silent: true,
            channel_name: "round22".to_string(),
            multimodal: MultimodalConfig::default(),
            multimodal_files: MultimodalFileConfig::default(),
            max_tool_iterations,
            on_delta,
            target_agent_id: Some("orchestrator".to_string()),
            visible_tool_names: None,
            extra_tools: Vec::new(),
            on_progress,
            origin: openhuman_core::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
        },
    )
    .await
    .map_err(|err| err.to_string())
}

#[tokio::test]
async fn no_progress_guard_uses_default_iteration_fallback_when_zero() {
    let provider = ScriptedProvider::new(vec![
        xml_tool_response("fail", json!({ "value": "one" })),
        xml_tool_response("fail", json!({ "value": "two" })),
        xml_tool_response("fail", json!({ "value": "three" })),
        xml_tool_response("fail", json!({ "value": "four" })),
        xml_tool_response("fail", json!({ "value": "five" })),
        xml_tool_response("fail", json!({ "value": "six" })),
    ]);

    let response = run_turn(
        provider.clone(),
        vec![Round22Tool::err("fail", "round22 failure")],
        0,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(response.text.contains("6 tool calls in a row failed"));
    assert!(response.text.contains("round22 failure:six"));
    assert_eq!(
        provider.requests().len(),
        6,
        "max_tool_iterations=0 should use the default cap, allowing the no-progress guard to halt first"
    );
}

#[tokio::test]
async fn hard_policy_block_repeat_halts_on_second_identical_call() {
    let provider = ScriptedProvider::new(vec![
        xml_tool_response("blocked", json!({ "value": "same" })),
        xml_tool_response("blocked", json!({ "value": "same" })),
    ]);
    let output = format!("{POLICY_BLOCKED_MARKER} read-only policy blocked this write");

    let response = run_turn(
        provider.clone(),
        vec![Round22Tool::err(
            "blocked",
            Box::leak(output.into_boxed_str()),
        )],
        8,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(response.text.contains("blocked by the security policy"));
    assert!(response.text.contains("re-issued with identical arguments"));
    assert_eq!(provider.requests().len(), 2);
}

#[tokio::test]
async fn glm_style_tool_call_executes_then_final_streams_in_chunks_and_progress() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(glm_response("browser_open/url>https://example.com/data")),
                Ok(text_response(
                    "This is a deliberately long final response from the scripted provider so the on_delta path emits more than one deterministic chunk for channel draft updates.",
                )),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        stream_events: vec![ProviderDelta::TextDelta {
            delta: "draft from provider".to_string(),
        }],
    });
    let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel(8);
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);

    let response = run_turn(
        provider.clone(),
        vec![Round22Tool::ok("shell", "shell-output")],
        4,
        Some(delta_tx),
        Some(progress_tx),
    )
    .await
    .unwrap();

    assert!(response
        .text
        .starts_with("This is a deliberately long final response"));
    let mut deltas = Vec::new();
    while let Ok(delta) = delta_rx.try_recv() {
        deltas.push(delta);
    }
    assert!(
        deltas.len() >= 2,
        "long final response should stream in at least two chunks, got {deltas:?}"
    );

    let mut progress = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        progress.push(event);
    }
    assert!(progress
        .iter()
        .any(|event| matches!(event, AgentProgress::TextDelta { delta, iteration: 1 } if delta == "draft from provider")));
    assert!(progress.iter().any(|event| matches!(
        event,
        AgentProgress::ToolCallCompleted {
            tool_name,
            success,
            ..
        } if tool_name == "shell" && *success
    )));
    assert!(progress
        .iter()
        .any(|event| matches!(event, AgentProgress::TurnCompleted { iterations: 2 })));

    let requests = provider.requests();
    assert!(requests.iter().all(|request| request.streamed));
    assert_eq!(requests[0].tool_names, vec!["shell"]);
    let second_request_text = requests[1]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(second_request_text.contains("curl -s 'https://example.com/data'"));
    assert!(second_request_text.contains("shell-output:curl -s 'https://example.com/data'"));
}
