use super::*;
use tinyagents_harness::events::{AgentEvent, EventListener, EventRecord, EventSink};

#[derive(Default)]
struct EventRecorder(std::sync::Mutex<Vec<AgentEvent>>);

impl EventListener for EventRecorder {
    fn on_event(&self, record: &EventRecord) {
        self.0.lock().unwrap().push(record.event.clone());
    }
}

/// A tool whose `display_label`/`display_detail` depend on the call
/// arguments — the shape a dynamic Composio/MCP/integration tool takes (e.g.
/// [`crate::integrations::composio::action_tool::ComposioActionTool`]'s
/// "Gmail send email"). Used to prove the bridge calls the tool's OWN
/// presentation methods instead of always deriving a label from the bare
/// tool name.
struct FakeLabeledTool;

#[async_trait::async_trait]
impl tinytools::Tool for FakeLabeledTool {
    fn name(&self) -> &str {
        "fake_send_email"
    }

    fn description(&self) -> &str {
        "sends an email (test double)"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("sent"))
    }

    fn display_label(&self, _args: &serde_json::Value) -> Option<String> {
        Some("Sending email".to_string())
    }

    fn display_detail(&self, args: &serde_json::Value) -> Option<String> {
        args.get("to").and_then(|v| v.as_str()).map(str::to_string)
    }
}

fn fake_tool_sets() -> Vec<Arc<Vec<Box<dyn tinytools::Tool>>>> {
    vec![Arc::new(vec![
        Box::new(FakeLabeledTool) as Box<dyn tinytools::Tool>
    ])]
}

#[tokio::test]
async fn bridge_forwards_tool_and_cost_progress() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "mock-model".to_string(),
    });
    sink.emit(AgentEvent::ToolStarted {
        parent_call_id: None,
        call_id: "c1".into(),
        tool_name: "echo".to_string(),
        input: None,
    });
    sink.emit(AgentEvent::ToolCompleted {
        parent_call_id: None,
        call_id: "c1".into(),
        tool_name: "echo".to_string(),
        started_at_ms: None,
        input: None,
        output: None,
        duration_ms: None,
        output_bytes: None,
        error: None,
        metadata: None,
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(100, 40),
    });

    let mut kinds = Vec::new();
    while let Ok(p) = rx.try_recv() {
        kinds.push(match p {
            AgentProgress::IterationStarted { .. } => "iter",
            AgentProgress::ToolCallStarted { .. } => "tool_start",
            AgentProgress::ToolCallCompleted { .. } => "tool_done",
            AgentProgress::TurnCostUpdated { input_tokens, .. } => {
                assert_eq!(input_tokens, 100);
                "cost"
            }
            _ => "other",
        });
    }
    assert!(kinds.contains(&"iter"));
    assert!(kinds.contains(&"tool_start"));
    assert!(kinds.contains(&"tool_done"));
    assert!(kinds.contains(&"cost"));

    let (input, output, _) = bridge.totals();
    assert_eq!((input, output), (100, 40));
}

fn run_started(id: &str) -> AgentEvent {
    AgentEvent::RunStarted {
        run_id: tinyagents_harness::ids::RunId::new(id),
        thread_id: None,
    }
}

/// A turn announces itself (`TurnStarted`, which the web channel turns into
/// `inference_start`) before anything else, exactly once. Without it a client
/// that did not send the turn — one started right after a Stop — never learns
/// a new turn is live and hides its stream until a reload.
#[tokio::test]
async fn the_top_level_run_announces_the_turn_once_and_first() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(run_started("root"));
    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "mock-model".to_string(),
    });
    // A retried or nested run on the same bridge is not a new turn.
    sink.emit(run_started("again"));

    let mut kinds = Vec::new();
    while let Ok(p) = rx.try_recv() {
        kinds.push(match p {
            AgentProgress::TurnStarted => "turn",
            AgentProgress::IterationStarted { .. } => "iter",
            _ => "other",
        });
    }
    assert_eq!(kinds.first(), Some(&"turn"), "{kinds:?}");
    assert_eq!(
        kinds.iter().filter(|k| **k == "turn").count(),
        1,
        "{kinds:?}"
    );
}

#[tokio::test]
async fn a_subagent_run_does_not_announce_a_turn() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::with_scope(
        Some(tx),
        "mock-model",
        "custom",
        10,
        Some(SubagentScope {
            agent_id: "researcher".into(),
            task_id: "t1".into(),
            extended_policy: false,
        }),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Vec::new(),
        crate::core::runtime::DispatchCtx::unbound(),
    );
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(run_started("child"));
    while let Ok(p) = rx.try_recv() {
        assert!(!matches!(p, AgentProgress::TurnStarted), "{p:?}");
    }
}

#[tokio::test]
async fn host_supplemental_generation_is_journaled_without_loop_progress_or_billing() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());
    let supplemental: tinyagents_harness::ids::CallId = "run-1-host-supplemental-1".into();
    sink.emit(AgentEvent::ModelStarted {
        call_id: supplemental.clone(),
        model: "mock-model".into(),
    });
    let mut cached_usage = Usage::new(50, 12);
    cached_usage.charged_amount = Some(tinyinference_llm::usage::ChargedAmount::usd_micros(8_000));
    sink.emit(AgentEvent::ModelResponseCached {
        call_id: supplemental.clone(),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: cached_usage,
    });
    sink.emit(AgentEvent::ModelCompleted {
        call_id: supplemental,
        started_at_ms: Some(1),
        usage: Some(Usage {
            charged_amount: Some(tinyinference_llm::usage::ChargedAmount::usd_micros(8_000)),
            ..Usage::new(50, 12)
        }),
        input: Some(serde_json::json!(["private prompt"])),
        output: Some(serde_json::json!("private response")),
    });

    let regular: tinyagents_harness::ids::CallId = "run-1-model-1".into();
    sink.emit(AgentEvent::ModelStarted {
        call_id: regular,
        model: "mock-model".into(),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(7, 3),
    });

    assert_eq!(bridge.totals(), (7, 3, 0.0));
    let progress: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(
        progress
            .iter()
            .filter(|event| matches!(event, AgentProgress::IterationStarted { .. }))
            .count(),
        1,
        "only the real harness call advances UI iteration state"
    );
    assert_eq!(
        progress
            .iter()
            .filter(|event| matches!(event, AgentProgress::TurnCostUpdated { .. }))
            .count(),
        1,
        "supplemental usage is folded by the host terminal sidecar, not emitted as duplicate live billing"
    );
}

#[tokio::test]
async fn model_completed_projects_generation_with_content_and_provider() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::with_scope(
        Some(tx),
        "chat-v1",
        "managed",
        10,
        None,
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Vec::new(),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ModelStarted {
        call_id: "m1".into(),
        model: "chat-v1".to_string(),
    });
    sink.emit(AgentEvent::ModelCompleted {
        call_id: "m1".into(),
        started_at_ms: None,
        usage: Some(Usage::new(1_000, 50)),
        input: Some(serde_json::json!([
            {"role": "system", "content": "You are OpenHuman."}
        ])),
        output: Some(serde_json::json!({"role": "assistant", "content": "hi"})),
    });

    let mut seen = None;
    while let Ok(p) = rx.try_recv() {
        if let AgentProgress::ModelCallCompleted {
            model,
            provider_id,
            subagent_task_id,
            input,
            output,
            input_tokens,
            cost_usd,
            ..
        } = p
        {
            seen = Some((
                model,
                provider_id,
                subagent_task_id,
                input,
                output,
                input_tokens,
                cost_usd,
            ));
        }
    }
    let (model, provider_id, task, input, output, input_tokens, cost_usd) =
        seen.expect("ModelCallCompleted projected from ModelCompleted");
    assert_eq!(model, "chat-v1");
    assert_eq!(provider_id, "managed");
    assert!(task.is_none(), "parent scope carries no task id");
    assert!(input.unwrap().to_string().contains("You are OpenHuman."));
    assert!(output.unwrap().to_string().contains("hi"));
    assert_eq!(input_tokens, 1_000);
    // chat-v1 is a managed tier handle with no catalogued list price, and this
    // call carried no reported charge: its cost is unknown, not a guessed
    // default rate (which once priced glm-5.3-flash at 14x what was billed).
    assert_eq!(cost_usd, None, "an unpriced call reports no cost");
}

#[tokio::test]
async fn subagent_model_completed_carries_task_attribution() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::with_scope(
        Some(tx),
        "burst-v1",
        "managed",
        8,
        Some(SubagentScope {
            agent_id: "researcher".to_string(),
            task_id: "ctx-1".to_string(),
            extended_policy: true,
        }),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Vec::new(),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());
    sink.emit(AgentEvent::ModelCompleted {
        call_id: "m1".into(),
        started_at_ms: None,
        usage: Some(Usage::new(10, 5)),
        input: None,
        output: None,
    });
    let mut task = None;
    while let Ok(p) = rx.try_recv() {
        if let AgentProgress::ModelCallCompleted {
            subagent_task_id, ..
        } = p
        {
            task = subagent_task_id;
        }
    }
    assert_eq!(
        task.as_deref(),
        Some("ctx-1"),
        "child model calls must carry the owning subagent task id"
    );
}

#[tokio::test]
async fn tool_completed_projects_output_arguments_and_elapsed() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ToolStarted {
        parent_call_id: None,
        call_id: "t1".into(),
        tool_name: "echo".to_string(),
        input: None,
    });
    sink.emit(AgentEvent::ToolCompleted {
        parent_call_id: None,
        call_id: "t1".into(),
        tool_name: "echo".to_string(),
        started_at_ms: None,
        input: Some(serde_json::json!({"text": "ping"})),
        output: Some(serde_json::Value::String("pong".to_string())),
        duration_ms: None,
        output_bytes: None,
        error: None,
        metadata: None,
    });

    let mut seen = None;
    while let Ok(p) = rx.try_recv() {
        if let AgentProgress::ToolCallCompleted {
            output,
            output_chars,
            arguments,
            ..
        } = p
        {
            seen = Some((output, output_chars, arguments));
        }
    }
    let (output, output_chars, arguments) = seen.expect("tool completion projected");
    assert_eq!(output, "pong");
    assert_eq!(output_chars, 4);
    assert!(arguments.unwrap().to_string().contains("ping"));
}

/// The answer the crate injects for an unknown tool (`unknown_tool_message`).
const UNKNOWN_TOOL_ANSWER: &str =
    "unknown tool `search_files` (arguments: {\"query\":\"config\"}): \
     no tool with that name is available to you, and calling it again will fail the same way.";

/// Emit what the crate emits for an unknown-tool call since TOOL-11: the typed
/// `UnknownToolCall`, then — through `recover_tool_call` — an ordinary
/// `ToolStarted`/`ToolCompleted` pair under the same call id whose result is
/// the corrective error text.
fn emit_recovered_unknown_tool_call(sink: &EventSink, call_id: &str) {
    sink.emit(AgentEvent::UnknownToolCall {
        call_id: call_id.into(),
        requested_name: "search_files".to_string(),
        arguments: serde_json::json!({ "query": "config" }),
        recovery: "tool_error".to_string(),
    });
    sink.emit(AgentEvent::ToolStarted {
        parent_call_id: None,
        call_id: call_id.into(),
        tool_name: "search_files".to_string(),
        input: Some(serde_json::json!({ "query": "config" })),
    });
    sink.emit(AgentEvent::ToolCompleted {
        parent_call_id: None,
        call_id: call_id.into(),
        tool_name: "search_files".to_string(),
        started_at_ms: None,
        input: Some(serde_json::json!({ "query": "config" })),
        output: Some(serde_json::Value::String(UNKNOWN_TOOL_ANSWER.to_string())),
        duration_ms: Some(0),
        output_bytes: Some(UNKNOWN_TOOL_ANSWER.len() as u64),
        error: Some(UNKNOWN_TOOL_ANSWER.to_string()),
        metadata: None,
    });
}

#[tokio::test]
include!("observability_tests_tool_rows.rs");
