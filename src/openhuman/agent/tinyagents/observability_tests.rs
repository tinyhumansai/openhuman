use super::*;
use tinyagents_harness::events::EventSink;

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
        call_id: "c1".into(),
        tool_name: "echo".to_string(),
    });
    sink.emit(AgentEvent::ToolCompleted {
        call_id: "c1".into(),
        tool_name: "echo".to_string(),
        started_at_ms: None,
        input: None,
        output: None,
        duration_ms: None,
        output_bytes: None,
        error: None,
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
    // chat-v1 is a managed tier handle — the tier-aware estimator must
    // price it (> $0); the old catalog-only lookup returned exactly 0.
    assert!(cost_usd > 0.0, "managed tier call must not price as $0");
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
            agent_id: "context_scout".to_string(),
            task_id: "ctx-1".to_string(),
            extended_policy: true,
        }),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
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
        call_id: "t1".into(),
        tool_name: "echo".to_string(),
    });
    sink.emit(AgentEvent::ToolCompleted {
        call_id: "t1".into(),
        tool_name: "echo".to_string(),
        started_at_ms: None,
        input: Some(serde_json::json!({"text": "ping"})),
        output: Some(serde_json::Value::String("pong".to_string())),
        duration_ms: None,
        output_bytes: None,
        error: None,
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

#[tokio::test]
async fn unknown_tool_call_projects_attempted_name_as_failed_timeline_row() {
    // #4118: the crate recovers an unavailable-tool call via ReturnToolError
    // without ever emitting Started/Completed for it. The bridge must still
    // surface the *attempted* tool on the timeline (a failed call) so the UI
    // shows what the agent tried, instead of the attempt vanishing.
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::UnknownToolCall {
        call_id: "c9".into(),
        requested_name: "search_files".to_string(),
        arguments: serde_json::json!({ "query": "config" }),
        recovery: "tool_error".to_string(),
    });

    let mut started_name = None;
    let mut completed: Option<(String, bool)> = None;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::ToolCallStarted { tool_name, .. } => started_name = Some(tool_name),
            AgentProgress::ToolCallCompleted {
                tool_name, success, ..
            } => completed = Some((tool_name, success)),
            _ => {}
        }
    }
    assert_eq!(
        started_name.as_deref(),
        Some("search_files"),
        "the attempted unavailable tool name must appear on the timeline"
    );
    assert_eq!(
        completed,
        Some(("search_files".to_string(), false)),
        "the attempted tool must be projected as a *failed* call"
    );
}

/// W2-budget-dedupe: two `UsageRecorded` events for the *same* model call
/// (as happens once the observe-only crate `BudgetMiddleware` re-emits usage
/// its `after_model` folded, on top of the runtime's own emit) must be
/// recorded into the bridge accounting **exactly once**. Without the dedupe
/// guard the totals would double.
#[tokio::test]
async fn duplicate_usage_for_same_model_call_is_recorded_once() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    // One model call → one `ModelStarted` (iteration cursor → 1).
    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "mock-model".to_string(),
    });
    // Same call surfaces usage twice (runtime emit + BudgetMiddleware re-emit).
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(100, 40),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(100, 40),
    });

    // Totals reflect a single record, not two.
    let (input, output, _) = bridge.totals();
    assert_eq!(
        (input, output),
        (100, 40),
        "the duplicate UsageRecorded for the same iteration must be skipped"
    );

    // Exactly one `TurnCostUpdated` footer emit for the call.
    let mut cost_updates = 0;
    while let Ok(p) = rx.try_recv() {
        if matches!(p, AgentProgress::TurnCostUpdated { .. }) {
            cost_updates += 1;
        }
    }
    assert_eq!(cost_updates, 1, "footer must update once per model call");

    // A genuinely new model call (iteration cursor → 2) records again.
    sink.emit(AgentEvent::ModelStarted {
        call_id: "c2".into(),
        model: "mock-model".to_string(),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(10, 5),
    });
    let (input, output, _) = bridge.totals();
    assert_eq!(
        (input, output),
        (110, 45),
        "a distinct model call (new iteration) must still record"
    );
}

// NOTE: the former `sentinel_tool_started_is_not_forwarded` test was removed
// here. The #4249 migration (commit 60097ba8d, "use sdk unknown tool
// recovery") deleted `UNKNOWN_TOOL_SENTINEL` + `UnknownToolRewriteMiddleware`
// in favour of the crate `UnknownToolPolicy::ReturnToolError` path, so a
// `ToolStarted` now only ever fires for real, model-visible tools (see the
// `ToolStarted` arm above — it no longer special-cases a sentinel). The test
// referenced the deleted constant (a stale reference reintroduced by a merge)
// and asserted behaviour that no longer exists.
