use async_trait::async_trait;
use openhuman_core::core::bus::BUS;
use openhuman_core::openhuman::agent::bus::{
    register_agent_handlers, AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD,
};
use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::config::{MultimodalConfig, MultimodalFileConfig};
use openhuman_core::openhuman::agent::messages::ChatMessage;
use openhuman_core::openhuman::security::POLICY_BLOCKED_MARKER;
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolContent, ToolResult, ToolScope};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use tinyinference::message::{AssistantMessage, ContentBlock, Message, MessageDelta};
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyinference::tool::ToolCall;
use tinyinference::usage::Usage;

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<Message>,
    tool_names: Vec<String>,
    streamed: bool,
}

#[derive(Default)]
struct ScriptedModel {
    responses: Mutex<VecDeque<anyhow::Result<ModelResponse>>>,
    requests: Mutex<Vec<CapturedRequest>>,
    stream_events: Vec<ModelStreamItem>,
}

impl ScriptedModel {
    fn new(responses: Vec<ModelResponse>) -> Arc<Self> {
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
impl ChatModel<()> for ScriptedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: OnceLock<ModelProfile> = OnceLock::new();
        Some(PROFILE.get_or_init(|| ModelProfile {
            provider: Some("round22".to_string()),
            tool_calling: true,
            parallel_tool_calls: true,
            ..ModelProfile::default()
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.capture(&request, false);
        self.pop_response()
    }

    async fn stream(&self, _state: &(), request: ModelRequest) -> tinyinference::Result<ModelStream> {
        self.capture(&request, true);
        let response = self.pop_response()?;
        let mut items = vec![ModelStreamItem::Started];
        items.extend(self.stream_events.iter().cloned());
        items.push(ModelStreamItem::Completed(response));
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

impl ScriptedModel {
    fn capture(&self, request: &ModelRequest, streamed: bool) {
        self.requests.lock().unwrap().push(CapturedRequest {
            messages: request.messages.clone(),
            tool_names: request.tools.iter().map(|tool| tool.name.clone()).collect(),
            streamed,
        });
    }

    fn pop_response(&self) -> tinyinference::Result<ModelResponse> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("script exhausted fallback")))
            .map_err(|error| tinyinference::Error::Model(error.to_string()))
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

fn text_response(text: &str) -> ModelResponse {
    let mut usage = Usage::new(3, 2);
    usage.cache_read_tokens = 1;
    ModelResponse::assistant(text).with_usage(usage)
}

fn tool_response(name: &str, args: serde_json::Value) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("before".to_string())],
            tool_calls: vec![ToolCall::new(format!("call-{name}"), name, args)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
            served_from_cache: false,
    }
}

fn browser_open_response(url: &str) -> ModelResponse {
    tool_response("shell", json!({ "command": format!("curl -s '{url}'") }))
}

async fn run_turn(
    model: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
) -> Result<AgentTurnResponse, String> {
    openhuman_core::core::bus::init().await.expect("bus init");
    register_agent_handlers();
    BUS.native().request::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        AgentTurnRequest {
            turn_model_source: openhuman_core::openhuman::agent::tinyagents::TurnModelSource::from_model(
                model,
            ),
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
    let provider = ScriptedModel::new(vec![
        tool_response("fail", json!({ "value": "one" })),
        tool_response("fail", json!({ "value": "two" })),
        tool_response("fail", json!({ "value": "three" })),
        tool_response("fail", json!({ "value": "four" })),
        tool_response("fail", json!({ "value": "five" })),
        tool_response("fail", json!({ "value": "six" })),
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
    let provider = ScriptedModel::new(vec![
        tool_response("blocked", json!({ "value": "same" })),
        tool_response("blocked", json!({ "value": "same" })),
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
    let provider = Arc::new(ScriptedModel {
        responses: Mutex::new(
            vec![
                Ok(browser_open_response("https://example.com/data")),
                Ok(text_response(
                    "This is a deliberately long final response from the scripted provider so the on_delta path emits more than one deterministic chunk for channel draft updates.",
                )),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        stream_events: vec![ModelStreamItem::MessageDelta(MessageDelta::text(
            "draft from provider",
        ))],
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
    // The raw `on_delta` Sender<String> path is retired (superseded by
    // `on_progress` text deltas — see `agent/bus.rs`), so its channel stays empty;
    // streaming is observed on `on_progress` below.
    let mut on_delta_chunks = Vec::new();
    while let Ok(delta) = delta_rx.try_recv() {
        on_delta_chunks.push(delta);
    }
    assert!(
        on_delta_chunks.is_empty(),
        "retired on_delta channel must stay empty, got {on_delta_chunks:?}"
    );

    let mut progress = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        progress.push(event);
    }
    // Streaming surfaces as `AgentProgress::TextDelta` on `on_progress`: each
    // streamed model call forwards the provider's delta, so a two-iteration turn
    // (tool round + final) emits at least two text deltas.
    let text_deltas = progress
        .iter()
        .filter(|event| matches!(event, AgentProgress::TextDelta { .. }))
        .count();
    assert!(
        text_deltas >= 2,
        "streaming should emit at least two on_progress text deltas, got {text_deltas}"
    );
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
        .map(Message::text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(second_request_text.contains("curl -s 'https://example.com/data'"));
    assert!(second_request_text.contains("shell-output:curl -s 'https://example.com/data'"));
}
