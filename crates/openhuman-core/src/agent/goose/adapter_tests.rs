use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::host::security_gate::{GateDecision, ToolCallRequest};
use tinyinference::{
    message::AssistantMessage,
    model::{ChatModel, ModelRequest, ModelResponse},
    tool::ToolCall,
    usage::Usage,
};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{
        messages::{ChatMessage, ConversationMessage},
        primary_orchestration::capability::{
            CapabilityAvailability, CapabilityBackend, CapabilityModality, CapabilityOperation,
            CapabilitySideEffect, MonetaryBoundary, ToolCapability, ToolRoute,
        },
        progress::AgentProgress,
    },
    tools::{Tool, ToolResult},
};

use super::{
    convert::{final_text, goose_to_openhuman},
    GooseCheckpointStore, GooseStopReason, GooseToolSecurity, GooseTurnAdapter,
    InMemoryGooseCheckpointStore,
};

pub(crate) struct ScriptedModel {
    pub(crate) responses: Mutex<VecDeque<ModelResponse>>,
    pub(crate) requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedModel {
    pub(crate) fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.responses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .ok_or_else(|| tinyinference::Error::Model("script exhausted".into()))
    }
}

pub(crate) struct CountingTool {
    pub(crate) calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "read_counter"
    }

    fn description(&self) -> &str {
        "Returns a deterministic counter value without external I/O."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"key": {"type": "string"}},
            "required": ["key"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success(format!("value:{}", args["key"])))
    }
}

pub(crate) struct NamedCountingTool {
    pub(crate) name: String,
    pub(crate) calls: Arc<AtomicUsize>,
}

impl NamedCountingTool {
    pub(crate) fn new(name: impl Into<String>, calls: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.into(),
            calls,
        }
    }
}

#[async_trait]
impl Tool for NamedCountingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Returns a deterministic counter value without external I/O."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"key": {"type": "string"}},
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let val = args.get("key").and_then(|k| k.as_str()).unwrap_or("ok");
        Ok(ToolResult::success(format!("{}:{}", self.name, val)))
    }
}

#[derive(Default)]
pub(crate) struct AllowSecurity;

#[async_trait]
impl GooseToolSecurity for AllowSecurity {
    async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
        Ok(GateDecision::Allow)
    }

    fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
}

pub(crate) fn tool_response(usage: Usage) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: Some("assistant-tool".into()),
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(
                "call-1",
                "read_counter",
                json!({"key": "alpha"}),
            )],
            usage: Some(usage),
        },
        usage: Some(usage),
        finish_reason: Some("tool_calls".into()),
        raw: None,
        resolved_model: None,
        continue_turn: Some("tool".into()),
        served_from_cache: false,
    }
}

pub(crate) fn final_response(text: &str, usage: Usage) -> ModelResponse {
    ModelResponse::assistant(text).with_usage(usage)
}

pub(crate) fn named_tool_response(
    call_id: &str,
    tool_name: &str,
    key_val: &str,
    usage: Usage,
) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: Some(format!("assistant-{call_id}")),
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(call_id, tool_name, json!({"key": key_val}))],
            usage: Some(usage),
        },
        usage: Some(usage),
        finish_reason: Some("tool_calls".into()),
        raw: None,
        resolved_model: None,
        continue_turn: Some("tool".into()),
        served_from_cache: false,
    }
}

pub(crate) fn test_route(name: &str, priority: u16, registration_index: usize) -> ToolRoute {
    ToolRoute::new(ToolCapability {
        name: name.to_string(),
        operations: vec![CapabilityOperation::ReadWorkspace],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::Local,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::LocalRead,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority,
        registration_index,
    })
}

pub(crate) fn default_route(name: &str) -> ToolRoute {
    test_route(name, 100, 0)
}

pub(crate) fn adapter(
    store: Arc<dyn GooseCheckpointStore>,
    model: Arc<dyn ChatModel<()>>,
    calls: Arc<AtomicUsize>,
    security: Arc<dyn GooseToolSecurity>,
    progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    cancel: CancellationToken,
) -> GooseTurnAdapter {
    adapter_with_snapshots_and_routes(
        store,
        model,
        Arc::new(vec![Box::new(CountingTool { calls })]),
        Arc::new(Vec::new()),
        vec![default_route("read_counter")],
        security,
        progress,
        cancel,
    )
}

pub(crate) fn adapter_with_snapshots_and_routes(
    store: Arc<dyn GooseCheckpointStore>,
    model: Arc<dyn ChatModel<()>>,
    durable_tools: Arc<Vec<Box<dyn Tool>>>,
    synthesized_tools: Arc<Vec<Box<dyn Tool>>>,
    routes: Vec<ToolRoute>,
    security: Arc<dyn GooseToolSecurity>,
    progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    cancel: CancellationToken,
) -> GooseTurnAdapter {
    GooseTurnAdapter {
        store,
        model,
        model_name: "mock-primary".into(),
        provider_id: "mock".into(),
        durable_tools,
        synthesized_tools,
        routes,
        security,
        progress,
        cancel,
        max_output_tokens: Some(64),
        max_primary_calls: 12,
        max_no_progress_calls: 2,
        contract: None,
    }
}

pub(crate) fn kickoff() -> Vec<ConversationMessage> {
    vec![ConversationMessage::Chat(ChatMessage::user("read alpha"))]
}

pub(crate) fn to_qwen_route(mut adapter: GooseTurnAdapter) -> GooseTurnAdapter {
    adapter.provider_id = "lmstudio".into();
    adapter.model_name = "qwen38-openhuman".into();
    adapter
}

#[test]
fn openhuman_message_shapes_round_trip_through_goose() {
    let source = vec![
        ConversationMessage::Chat(ChatMessage::system("system")),
        ConversationMessage::Chat(ChatMessage::user("hello")),
        ConversationMessage::AssistantToolCalls {
            text: Some("checking".into()),
            tool_calls: vec![crate::inference::provider::ToolCall {
                id: "call-1".into(),
                name: "read_counter".into(),
                arguments: json!({"key": "alpha"}).to_string(),
                extra_content: None,
            }],
            reasoning_content: Some("private".into()),
            extra_metadata: Some(json!({"source": "fixture"})),
        },
        ConversationMessage::ToolResults(vec![crate::agent::messages::ToolResultMessage {
            tool_call_id: "call-1".into(),
            content: "value:alpha".into(),
        }]),
        ConversationMessage::Chat(ChatMessage::assistant("done")),
    ];
    let checkpoint = GooseTurnAdapter::checkpoint_from_openhuman(&source).unwrap();
    let roundtrip = goose_to_openhuman(&checkpoint.conversation);
    assert_eq!(
        serde_json::to_value(roundtrip).unwrap(),
        serde_json::to_value(source).unwrap()
    );
    assert!(checkpoint.actions["call-1"].observation.is_some());
}

#[tokio::test]
async fn first_model_request_advertises_tools_in_route_order_even_when_snapshot_order_differs() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "route-order",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls_first = Arc::new(AtomicUsize::new(0));
    let calls_second = Arc::new(AtomicUsize::new(0));

    // Snapshot order has "tool_second" first, then "tool_first".
    let durable_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(NamedCountingTool::new("tool_second", calls_second.clone())),
        Box::new(NamedCountingTool::new("tool_first", calls_first.clone())),
    ]);
    let synthesized_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(Vec::new());

    // Route order explicitly places "tool_first" before "tool_second".
    let routes = vec![
        test_route("tool_first", 10, 0),
        test_route("tool_second", 20, 1),
    ];

    let model = Arc::new(ScriptedModel::new(vec![final_response(
        "ok",
        Usage::new(10, 2),
    )]));

    let outcome = adapter_with_snapshots_and_routes(
        store,
        model.clone(),
        durable_tools,
        synthesized_tools,
        routes,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("route-order")
    .await
    .unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let advertised = &requests[0].tools;
    assert_eq!(advertised.len(), 2);
    assert_eq!(advertised[0].name, "tool_first");
    assert_eq!(advertised[1].name, "tool_second");
}

#[tokio::test]
async fn tool_present_in_either_snapshot_but_omitted_from_routes_is_neither_advertised_nor_executable(
) {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "omitted-durable",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls_durable = Arc::new(AtomicUsize::new(0));
    let calls_synth = Arc::new(AtomicUsize::new(0));
    let calls_routed = Arc::new(AtomicUsize::new(0));

    // "omitted_durable" in durable snapshot, "omitted_synth" in synthesized snapshot.
    let durable_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(NamedCountingTool::new(
            "omitted_durable",
            calls_durable.clone(),
        )),
        Box::new(NamedCountingTool::new("routed_tool", calls_routed.clone())),
    ]);
    let synthesized_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(
        NamedCountingTool::new("omitted_synth", calls_synth.clone()),
    )]);

    // Only "routed_tool" is in routes.
    let routes = vec![test_route("routed_tool", 10, 0)];

    let model = Arc::new(ScriptedModel::new(vec![named_tool_response(
        "call-omitted",
        "omitted_durable",
        "val",
        Usage::new(10, 2),
    )]));

    let adapter = adapter_with_snapshots_and_routes(
        store.clone(),
        model.clone(),
        durable_tools.clone(),
        synthesized_tools.clone(),
        routes.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );

    let outcome = adapter.run("omitted-durable").await.unwrap();
    assert_eq!(outcome.stop_reason, GooseStopReason::UnavailableTool);
    assert!(outcome.checkpoint.actions.contains_key("call-omitted"));
    let action = &outcome.checkpoint.actions["call-omitted"];
    assert!(action.observation.is_none());
    assert_eq!(calls_durable.load(Ordering::SeqCst), 0);
    assert_eq!(calls_synth.load(Ordering::SeqCst), 0);
    assert_eq!(calls_routed.load(Ordering::SeqCst), 0);

    // Verify advertising: neither omitted durable nor omitted synth tool is advertised.
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tools.len(), 1);
    assert_eq!(requests[0].tools[0].name, "routed_tool");
    // Also verify omitted synthesized tool execution fails.
    store.insert(
        "omitted-synth",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model_synth = Arc::new(ScriptedModel::new(vec![named_tool_response(
        "call-synth",
        "omitted_synth",
        "val",
        Usage::new(10, 2),
    )]));
    let adapter_synth = adapter_with_snapshots_and_routes(
        store,
        model_synth,
        durable_tools,
        synthesized_tools,
        routes,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );
    let outcome_synth = adapter_synth.run("omitted-synth").await.unwrap();
    assert_eq!(outcome_synth.stop_reason, GooseStopReason::UnavailableTool);
    assert!(outcome_synth.checkpoint.actions.contains_key("call-synth"));
    let action_synth = &outcome_synth.checkpoint.actions["call-synth"];
    assert!(action_synth.observation.is_none());
    assert_eq!(calls_synth.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejected_unplanned_call_reaches_neither_security_authorization_nor_execution() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "unplanned",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let auth_calls = Arc::new(AtomicUsize::new(0));

    struct SpySecurity {
        auth_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GooseToolSecurity for SpySecurity {
        async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
            self.auth_calls.fetch_add(1, Ordering::SeqCst);
            Ok(GateDecision::Allow)
        }

        fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
    }

    let durable_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(NamedCountingTool::new(
        "unplanned_tool",
        calls.clone(),
    ))]);
    let synthesized_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(Vec::new());
    // Empty routes: "unplanned_tool" is unplanned.
    let routes = Vec::new();

    let model = Arc::new(ScriptedModel::new(vec![named_tool_response(
        "call-unplanned",
        "unplanned_tool",
        "val",
        Usage::new(10, 2),
    )]));

    let adapter = adapter_with_snapshots_and_routes(
        store,
        model,
        durable_tools,
        synthesized_tools,
        routes,
        Arc::new(SpySecurity {
            auth_calls: auth_calls.clone(),
        }),
        None,
        CancellationToken::new(),
    );

    let outcome = adapter.run("unplanned").await.unwrap();
    assert_eq!(outcome.stop_reason, GooseStopReason::UnavailableTool);
    assert!(outcome.checkpoint.actions.contains_key("call-unplanned"));
    let action = &outcome.checkpoint.actions["call-unplanned"];
    assert!(action.observation.is_none());
    assert_eq!(auth_calls.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn terminal_route_failure_classification() {
    use super::tools::is_terminal_route_failure;
    assert!(is_terminal_route_failure(
        "USER_INSUFFICIENT_CREDITS: balance is zero"
    ));
    assert!(is_terminal_route_failure(
        "This request requires more credits"
    ));
    assert!(is_terminal_route_failure("Insufficient Balance"));
    assert!(is_terminal_route_failure("Quota exceeded for model"));
    assert!(is_terminal_route_failure(
        "No active credentials for provider: custom_openai"
    ));
    assert!(is_terminal_route_failure(
        "invalid_authentication_error: key revoked"
    ));
    assert!(!is_terminal_route_failure("File not found: test.txt"));
    assert!(!is_terminal_route_failure("Network timeout after 5000ms"));
}

#[test]
fn same_boundary_alternative_retention_rules() {
    use super::tools::{is_same_boundary_alternative, retain_authorized_alternatives};
    use crate::agent::primary_orchestration::capability::*;

    let search_free_a = ToolRoute::new(ToolCapability {
        name: "search_a".into(),
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::DirectNetwork,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::ExternalRead,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority: 100,
        registration_index: 0,
    });

    let search_free_b = ToolRoute::new(ToolCapability {
        name: "search_b".into(),
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::DirectNetwork,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::ExternalRead,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority: 90,
        registration_index: 1,
    });

    let search_paid = ToolRoute::new(ToolCapability {
        name: "search_paid".into(),
        operations: vec![CapabilityOperation::SearchWeb],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::Managed,
        monetary_boundary: MonetaryBoundary::ManagedMetered,
        side_effect: CapabilitySideEffect::ExternalRead,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority: 50,
        registration_index: 2,
    });

    let shell_tool = ToolRoute::new(ToolCapability {
        name: "shell".into(),
        operations: vec![CapabilityOperation::ExecuteCommand],
        modalities: vec![CapabilityModality::Text],
        backend: CapabilityBackend::Local,
        monetary_boundary: MonetaryBoundary::NonMetered,
        side_effect: CapabilitySideEffect::LocalWrite,
        availability: CapabilityAvailability::Available,
        permission: Default::default(),
        priority: 100,
        registration_index: 3,
    });

    // Same boundary matches
    assert!(is_same_boundary_alternative(&search_free_a, &search_free_b));
    // Different monetary boundary fails
    assert!(!is_same_boundary_alternative(&search_free_a, &search_paid));
    // Different operation fails
    assert!(!is_same_boundary_alternative(&search_free_a, &shell_tool));

    let all_routes = vec![
        search_free_a.clone(),
        search_free_b.clone(),
        search_paid.clone(),
    ];
    let retained = retain_authorized_alternatives(&all_routes, &["search_a".to_string()]);
    // search_b is retained, but search_a is excluded and search_paid (different monetary boundary) is excluded
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].capability.name, "search_b");
}

pub(crate) struct TerminalFailingTool {
    pub(crate) name: String,
    pub(crate) error_msg: String,
}

#[async_trait]
impl Tool for TerminalFailingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "A tool that fails with a terminal error"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "key": { "type": "string" } }
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::error(&self.error_msg))
    }
}

#[tokio::test]
async fn terminal_route_failure_marks_route_unavailable_and_blocks_subsequent_request() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn-terminal",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        named_tool_response("call-1", "credit_failing_tool", "arg1", Usage::new(10, 2)),
        named_tool_response("call-2", "credit_failing_tool", "arg2", Usage::new(10, 2)),
    ]));
    let failing_tool = Box::new(TerminalFailingTool {
        name: "credit_failing_tool".into(),
        error_msg: "USER_INSUFFICIENT_CREDITS: balance is zero".into(),
    });
    let adapter = adapter_with_snapshots_and_routes(
        store.clone(),
        model,
        Arc::new(vec![failing_tool]),
        Arc::new(Vec::new()),
        vec![default_route("credit_failing_tool")],
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );

    let outcome = adapter.run("turn-terminal").await.unwrap();
    // After call-1 failed with terminal error, credit_failing_tool was marked unavailable.
    // Call-2 then tried to request credit_failing_tool, which was stopped by UnavailableRequestGuard!
    assert_eq!(outcome.stop_reason, GooseStopReason::UnavailableTool);
    assert!(outcome
        .checkpoint
        .unavailable_routes
        .contains(&"credit_failing_tool".to_string()));
}
