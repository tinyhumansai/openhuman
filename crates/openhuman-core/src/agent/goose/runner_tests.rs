use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::host::security_gate::{GateDecision, ToolCallRequest};
use tinyinference::{model::ModelResponse, usage::Usage};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{
        messages::{ChatMessage, ConversationMessage},
        progress::AgentProgress,
    },
    tools::{Tool, ToolResult},
};

use super::{
    adapter_tests::*, GooseCheckpointStore, GooseStopReason, GooseToolSecurity, GooseTurnAdapter,
    InMemoryGooseCheckpointStore,
};

#[tokio::test]
async fn state_machine_persists_action_before_execution_and_one_observation() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        tool_response(Usage::new(10, 2)),
        final_response("finished", Usage::new(20, 3)),
    ]));
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = adapter(
        store.clone(),
        model.clone(),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("turn")
    .await
    .unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.checkpoint.actions.len(), 1);
    assert_eq!(
        outcome.checkpoint.actions["call-1"]
            .observation
            .as_ref()
            .unwrap()
            .output,
        "value:\"alpha\""
    );
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert_eq!(
        model.requests.lock().unwrap()[0].tools[0].name,
        "read_counter"
    );

    let resumed = adapter(
        store,
        model,
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("turn")
    .await
    .unwrap();
    assert_eq!(resumed.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn primary_call_ceiling_yields_a_resumable_checkpoint() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "ceiling",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![tool_response(Usage::new(10, 2))]));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut bounded = adapter(
        store,
        model.clone(),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );
    bounded.max_primary_calls = 1;

    let outcome = bounded.run("ceiling").await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::CallCeiling);
    assert_eq!(outcome.checkpoint.usage.primary_calls, 1);
    assert!(outcome.checkpoint.is_resumable());
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct DenySecurity;

#[async_trait]
impl GooseToolSecurity for DenySecurity {
    async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
        Ok(GateDecision::deny("fixture policy denied this call"))
    }

    fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
}

#[tokio::test]
async fn authorization_denial_is_a_single_observation_and_never_executes() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "denied",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = adapter(
        store,
        Arc::new(ScriptedModel::new(vec![
            tool_response(Usage::new(10, 2)),
            final_response("cannot run that", Usage::new(20, 3)),
        ])),
        calls.clone(),
        Arc::new(DenySecurity),
        None,
        CancellationToken::new(),
    )
    .run("denied")
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let observation = outcome.checkpoint.actions["call-1"]
        .observation
        .as_ref()
        .unwrap();
    assert!(!observation.success);
    assert_eq!(observation.output, "fixture policy denied this call");
}

#[tokio::test]
async fn latest_context_occupancy_is_not_cumulative_traffic() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "usage",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        tool_response(Usage::new(100, 5)),
        final_response("finished", Usage::new(140, 7)),
    ]));
    let outcome = adapter(
        store,
        model,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("usage")
    .await
    .unwrap();
    assert_eq!(outcome.checkpoint.usage.primary_calls, 2);
    assert_eq!(outcome.checkpoint.usage.latest_primary_input_tokens, 140);
    assert_eq!(outcome.checkpoint.usage.cumulative_input_tokens, 240);
    assert_eq!(outcome.checkpoint.usage.cumulative_output_tokens, 12);
}

struct WaitingSecurity {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl GooseToolSecurity for WaitingSecurity {
    async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(GateDecision::Prompted { approved: true })
    }

    fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
}

#[tokio::test]
async fn approval_wait_occurs_after_action_checkpoint_and_before_execution() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "approval",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let security = Arc::new(WaitingSecurity {
        entered: entered.clone(),
        release: release.clone(),
    });
    let approval_adapter = adapter(
        store.clone(),
        Arc::new(ScriptedModel::new(vec![
            tool_response(Usage::new(10, 2)),
            final_response("approved", Usage::new(20, 2)),
        ])),
        calls.clone(),
        security,
        None,
        CancellationToken::new(),
    );
    let turn = tokio::spawn(async move { approval_adapter.run("approval").await });
    entered.notified().await;
    let waiting = store.load("approval").await.unwrap();
    assert!(waiting.actions.contains_key("call-1"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    release.notify_one();
    assert_eq!(
        turn.await.unwrap().unwrap().stop_reason,
        GooseStopReason::FinalAnswer
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_persists_one_matching_observation_without_execution() {
    let source = vec![
        ConversationMessage::Chat(ChatMessage::user("read alpha")),
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![crate::inference::provider::ToolCall {
                id: "call-1".into(),
                name: "read_counter".into(),
                arguments: json!({"key": "alpha"}).to_string(),
                extra_content: None,
            }],
            reasoning_content: None,
            extra_metadata: None,
        },
    ];
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "cancel",
        GooseTurnAdapter::checkpoint_from_openhuman(&source).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let outcome = adapter(
        store,
        Arc::new(ScriptedModel::new(Vec::new())),
        calls.clone(),
        Arc::new(AllowSecurity),
        None,
        cancel,
    )
    .run("cancel")
    .await
    .unwrap();
    assert_eq!(outcome.stop_reason, GooseStopReason::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let observation = outcome.checkpoint.actions["call-1"]
        .observation
        .as_ref()
        .unwrap();
    assert!(!observation.success);
    assert!(observation.output.contains("cancelled"));
}

#[tokio::test]
async fn progress_projects_model_tool_usage_and_turn_lifecycle() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "progress",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    adapter(
        store,
        Arc::new(ScriptedModel::new(vec![
            tool_response(Usage::new(10, 2)),
            final_response("done", Usage::new(20, 3)),
        ])),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AllowSecurity),
        Some(tx),
        CancellationToken::new(),
    )
    .run("progress")
    .await
    .unwrap();
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert!(matches!(events.first(), Some(AgentProgress::TurnStarted)));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentProgress::ModelCallCompleted { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentProgress::ToolCallStarted { .. }))
            .count(),
        1
    );
    assert!(matches!(
        events.last(),
        Some(AgentProgress::TurnCompleted { iterations: 2 })
    ));
}

#[tokio::test]
async fn denied_planned_route_cannot_execute() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "denied-planned",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let auth_calls = Arc::new(AtomicUsize::new(0));

    struct DenyingSpySecurity {
        auth_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GooseToolSecurity for DenyingSpySecurity {
        async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
            self.auth_calls.fetch_add(1, Ordering::SeqCst);
            Ok(GateDecision::deny("planned route was denied by policy"))
        }

        fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
    }

    // Tool is planned in routes.
    let durable_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(NamedCountingTool::new(
        "planned_tool",
        calls.clone(),
    ))]);
    let synthesized_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(Vec::new());
    let routes = vec![test_route("planned_tool", 10, 0)];

    let model = Arc::new(ScriptedModel::new(vec![
        named_tool_response("call-planned", "planned_tool", "val", Usage::new(10, 2)),
        final_response("cannot run that", Usage::new(20, 3)),
    ]));

    let outcome = adapter_with_snapshots_and_routes(
        store,
        model,
        durable_tools,
        synthesized_tools,
        routes,
        Arc::new(DenyingSpySecurity {
            auth_calls: auth_calls.clone(),
        }),
        None,
        CancellationToken::new(),
    )
    .run("denied-planned")
    .await
    .unwrap();

    // Authorization was reached for the planned route.
    assert_eq!(auth_calls.load(Ordering::SeqCst), 1);
    // Tool execution was refused.
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // Denial observation was durably recorded.
    let observation = outcome.checkpoint.actions["call-planned"]
        .observation
        .as_ref()
        .unwrap();
    assert!(!observation.success);
    assert_eq!(observation.output, "planned route was denied by policy");
}

struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &'static str {
        "failing_tool"
    }

    fn description(&self) -> &'static str {
        "A tool that always returns an error"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string" }
            }
        })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::error("persistent server failure"))
    }
}

#[tokio::test]
async fn loop_guard_unavailable_tool_stops_and_records_reason() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![named_tool_response(
        "call-1",
        "non_existent_tool",
        "test",
        Usage::new(10, 2),
    )]));
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = adapter(
        store.clone(),
        model,
        calls,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("turn")
    .await
    .unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::UnavailableTool);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("unavailable_tool")
    );
    let final_text = super::convert::final_text(&outcome.checkpoint.conversation).unwrap();
    assert!(
        final_text.contains("not in active routes"),
        "Expected route explanation, got: {final_text}"
    );
}

#[tokio::test]
async fn loop_guard_duplicate_signature_stops_and_records_reason() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        named_tool_response("call-1", "read_counter", "same_arg", Usage::new(10, 2)),
        named_tool_response("call-2", "read_counter", "same_arg", Usage::new(10, 2)),
    ]));
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = adapter(
        store.clone(),
        model,
        calls,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("turn")
    .await
    .unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::DuplicateSignature);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("duplicate_signature")
    );
    let final_text = super::convert::final_text(&outcome.checkpoint.conversation).unwrap();
    assert!(
        final_text.contains("duplicate call signature"),
        "Expected duplicate explanation, got: {final_text}"
    );
}

#[tokio::test]
async fn loop_guard_repeated_failure_stops_and_records_reason() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        named_tool_response("call-1", "failing_tool", "arg1", Usage::new(10, 2)),
        named_tool_response("call-2", "failing_tool", "arg2", Usage::new(10, 2)),
    ]));
    let adapter = adapter_with_snapshots_and_routes(
        store.clone(),
        model,
        Arc::new(vec![Box::new(FailingTool)]),
        Arc::new(Vec::new()),
        vec![default_route("failing_tool")],
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );
    let outcome = adapter.run("turn").await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::RepeatedFailure);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("repeated_failure")
    );
    assert_eq!(outcome.checkpoint.repeated_failure_count, 2);
    let final_text = super::convert::final_text(&outcome.checkpoint.conversation).unwrap();
    assert!(
        final_text.contains("failed repeatedly"),
        "Expected failure explanation, got: {final_text}"
    );
}

#[tokio::test]
async fn loop_guard_no_progress_stops_and_records_reason() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![
        named_tool_response("call-1", "read_counter", "arg1", Usage::new(10, 2)),
        named_tool_response("call-2", "read_counter", "arg2", Usage::new(10, 2)),
    ]));
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = adapter(
        store.clone(),
        model,
        calls,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    )
    .run("turn")
    .await
    .unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::NoProgress);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("no_progress")
    );
    assert_eq!(outcome.checkpoint.no_progress_count, 2);
    let final_text = super::convert::final_text(&outcome.checkpoint.conversation).unwrap();
    assert!(
        final_text.contains("without progress"),
        "Expected no-progress explanation, got: {final_text}"
    );
}

#[tokio::test]
async fn loop_guard_completion_contract_satisfied_records_completed() {
    let store = Arc::new(InMemoryGooseCheckpointStore::default());
    store.insert(
        "turn",
        GooseTurnAdapter::checkpoint_from_openhuman(&kickoff()).unwrap(),
    );
    let model = Arc::new(ScriptedModel::new(vec![final_response(
        "Here is the answer to your question.",
        Usage::new(15, 3),
    )]));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapter = adapter(
        store.clone(),
        model,
        calls,
        Arc::new(AllowSecurity),
        None,
        CancellationToken::new(),
    );
    adapter.contract = Some(crate::agent::primary_orchestration::CompletionContract::Chat);

    let outcome = adapter.run("turn").await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("completed")
    );
    assert_eq!(
        outcome.checkpoint.completion_state,
        Some(crate::agent::primary_orchestration::CompletionStatus::Complete)
    );
}
