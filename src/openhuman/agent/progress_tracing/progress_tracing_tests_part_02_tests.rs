use super::*;

// ── identity / attribution / content capture ───────────────────────────────

#[test]
fn turn_span_carries_agent_client_and_source_attribution() {
    let mut c = SpanCollector::new(
        TraceContext::new("trace:req-9", Some("user-123".to_string()))
            .with_client_id("socket-abc")
            .with_agent_id("researcher")
            .with_channel_source("autonomous"),
    );
    c.record(&AgentProgress::TurnStarted, 0);
    // Trace name folds in the agent id.
    let turn = find(c.spans(), "agent.turn:researcher");
    assert_eq!(turn.kind, SpanKind::Turn);
    // Real user id is the user attribution; the transport client id is a
    // separate attribute, never conflated with the user.
    assert_eq!(turn.attributes["user.id"], serde_json::json!("user-123"));
    assert_eq!(
        turn.attributes["client.id"],
        serde_json::json!("socket-abc")
    );
    assert_eq!(turn.attributes["agent.id"], serde_json::json!("researcher"));
    assert_eq!(
        turn.attributes["channel.source"],
        serde_json::json!("autonomous")
    );
}

#[test]
fn turn_span_name_stays_plain_without_agent_id() {
    let mut c = SpanCollector::new(ctx());
    c.record(&AgentProgress::TurnStarted, 0);
    assert!(names(c.spans()).contains(&"agent.turn".to_string()));
}

#[test]
fn thread_id_falls_back_to_trace_id_without_session_group() {
    // Every trace must end up with a Langfuse sessionId: with no explicit
    // session group, the trace id itself is stamped as thread.id.
    let mut c = SpanCollector::new(TraceContext::new("sess-42:req-1", None));
    c.record(&AgentProgress::TurnStarted, 0);
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes["thread.id"],
        serde_json::json!("sess-42:req-1")
    );
}

#[test]
fn tool_io_is_captured_when_capture_content_is_on() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    c.record(&tool_started("c1", "web_search", 1), 1);
    let tool = find(c.spans(), "tool.web_search");
    let input = tool.input.as_ref().and_then(|v| v.as_str()).unwrap();
    assert!(
        input.contains("do-not-export"),
        "tool arguments must be recorded as span input when capture is on"
    );

    // Subagent tool result → span output.
    c.record(&spawn("task-1", "Researcher"), 2);
    c.record(
        &AgentProgress::SubagentToolCallStarted {
            agent_id: "researcher".to_string(),
            task_id: "task-1".to_string(),
            call_id: "sc-1".to_string(),
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "notes.md"}),
            iteration: 1,
            display_label: None,
            display_detail: None,
        },
        3,
    );
    c.record(
        &AgentProgress::SubagentToolCallCompleted {
            agent_id: "researcher".to_string(),
            task_id: "task-1".to_string(),
            call_id: "sc-1".to_string(),
            tool_name: "read_file".to_string(),
            success: true,
            output_chars: 13,
            output: "file contents".to_string(),
            arguments: None,
            elapsed_ms: 4,
            iteration: 1,
            failure: None,
        },
        4,
    );
    let child_tool = find(c.spans(), "tool.read_file");
    assert!(child_tool
        .input
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap()
        .contains("notes.md"));
    assert_eq!(
        child_tool.output.as_ref().and_then(|v| v.as_str()),
        Some("file contents")
    );
}

#[test]
fn tool_io_is_never_recorded_when_capture_content_is_off() {
    // Default ctx() has capture_content = false.
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (tool_started("c1", "web_search", 1), 1),
        (spawn("task-1", "Researcher"), 2),
        (
            AgentProgress::SubagentToolCallStarted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "secret.md"}),
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            3,
        ),
        (
            AgentProgress::SubagentToolCallCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                call_id: "sc-1".to_string(),
                tool_name: "read_file".to_string(),
                success: true,
                output_chars: 6,
                output: "sekrit".to_string(),
                arguments: None,
                elapsed_ms: 4,
                iteration: 1,
                failure: None,
            },
            4,
        ),
    ]);
    c.finish(100);
    for span in c.spans() {
        if span.kind == SpanKind::Tool {
            assert!(span.input.is_none(), "no tool input with capture off");
            assert!(span.output.is_none(), "no tool output with capture off");
        }
    }
    let blob = serde_json::to_string(c.spans()).unwrap();
    assert!(!blob.contains("do-not-export"));
    assert!(!blob.contains("sekrit"));
}

#[test]
fn captured_tool_io_is_truncated_with_marker() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    let huge = "x".repeat(10_000);
    c.record(
        &AgentProgress::ToolCallStarted {
            call_id: "c1".to_string(),
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({ "cmd": huge }),
            iteration: 1,
            display_label: None,
            display_detail: None,
        },
        1,
    );
    let tool = find(c.spans(), "tool.shell");
    let input = tool.input.as_ref().and_then(|v| v.as_str()).unwrap();
    assert!(input.contains("[truncated"), "marker must flag truncation");
    assert!(
        input.chars().count() < 4_100,
        "captured input must be capped near 4000 chars, got {}",
        input.chars().count()
    );
}

#[test]
fn model_call_completed_emits_generation_span_with_usage_cost_and_pricing() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 1_000),
        (
            AgentProgress::IterationStarted {
                iteration: 1,
                max_iterations: 5,
            },
            1_010,
        ),
        (model_call("agentic-v1", 0, 0), 1_500),
    ]);
    c.finish(2_000);
    let spans = c.spans();

    let generation = find(spans, "llm.agentic-v1");
    assert_eq!(generation.kind, SpanKind::Generation);
    // Parented under the live iteration; starts at the iteration start
    // (ModelStarted) and ends when the usage record was observed.
    let iter = find(spans, "agent.iteration#1");
    assert_eq!(
        generation.parent_span_id.as_deref(),
        Some(iter.span_id.as_str())
    );
    assert_eq!(generation.start_unix_ms, 1_010);
    assert_eq!(generation.end_unix_ms, Some(1_500));
    assert_eq!(generation.status, SpanStatus::Ok);

    let a = &generation.attributes;
    // Provider-labeled model: `{provider_id}.{model}`.
    assert_eq!(
        a["gen_ai.request.model"],
        serde_json::json!("managed.agentic-v1")
    );
    assert_eq!(a["gen_ai.usage.input_tokens"], serde_json::json!(1_000));
    assert_eq!(a["gen_ai.usage.output_tokens"], serde_json::json!(200));
    // Cache reads always flow, even when other calls happen to be zero.
    assert_eq!(
        a["gen_ai.usage.cached_input_tokens"],
        serde_json::json!(300)
    );
    assert_eq!(a["gen_ai.usage.cost_usd"], serde_json::json!(0.0042));
    // Managed tier handle → managed provenance.
    assert_eq!(a["gen_ai.provider"], serde_json::json!("managed"));
    // Pricing basis is auditable.
    assert_eq!(
        a["gen_ai.pricing.input_per_mtok_usd"],
        serde_json::json!(0.435)
    );
    assert!(a.get("gen_ai.pricing.output_per_mtok_usd").is_some());
    // Zero reasoning / cache-write tokens are omitted on the generation.
    assert!(a.get("gen_ai.usage.reasoning_tokens").is_none());
    assert!(a.get("gen_ai.usage.cache_creation_tokens").is_none());
}

#[test]
fn custom_model_generation_is_stamped_custom_provenance() {
    // A BYO model rides whatever provider id the event carries ("openai",
    // "ollama", or the "custom" default) — both on the generation and the root.
    let event = AgentProgress::ModelCallCompleted {
        model: "claude-imaginary-9".to_string(),
        provider_id: "custom".to_string(),
        subagent_task_id: None,
        input: None,
        output: None,
        iteration: 1,
        input_tokens: 10,
        output_tokens: 2,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        cost_usd: 0.0001,
    };
    let mut c = collect(&[(AgentProgress::TurnStarted, 0), (event, 10)]);
    c.finish(20);
    let generation = find(c.spans(), "llm.claude-imaginary-9");
    assert_eq!(
        generation.attributes["gen_ai.provider"],
        serde_json::json!("custom")
    );
    assert_eq!(
        generation.attributes["gen_ai.request.model"],
        serde_json::json!("custom.claude-imaginary-9")
    );
    // Provenance also lands on the root turn span (→ trace metadata).
    let turn = find(c.spans(), "agent.turn");
    assert_eq!(
        turn.attributes["gen_ai.provider"],
        serde_json::json!("custom")
    );
}

#[test]
fn reasoning_and_cache_write_tokens_flow_to_generation_and_root_rollup() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call("agentic-v1", 128, 64), 10),
        (model_call("agentic-v1", 72, 0), 20),
    ]);
    c.finish(30);
    let spans = c.spans();

    // Per-call values on the generations.
    let generations: Vec<&TraceSpan> = spans
        .iter()
        .filter(|s| s.kind == SpanKind::Generation)
        .collect();
    assert_eq!(generations.len(), 2, "one generation per model call");
    assert_eq!(
        generations[0].attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(128)
    );
    assert_eq!(
        generations[0].attributes["gen_ai.usage.cache_creation_tokens"],
        serde_json::json!(64)
    );
    assert_eq!(
        generations[1].attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(72)
    );

    // Cumulative rollup on the root turn span (TurnCostUpdated doesn't carry
    // these dimensions).
    let turn = find(spans, "agent.turn");
    assert_eq!(
        turn.attributes["gen_ai.usage.reasoning_tokens"],
        serde_json::json!(200)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.cache_creation_tokens"],
        serde_json::json!(64)
    );
}

#[test]
fn zero_reasoning_turn_leaves_root_without_reasoning_attr() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call("agentic-v1", 0, 0), 10),
    ]);
    c.finish(20);
    let turn = find(c.spans(), "agent.turn");
    assert!(turn
        .attributes
        .get("gen_ai.usage.reasoning_tokens")
        .is_none());
    assert!(turn
        .attributes
        .get("gen_ai.usage.cache_creation_tokens")
        .is_none());
}

// ── run-type classification ─────────────────────────────────────────────────

#[test]
fn run_type_classifies_known_sources() {
    assert_eq!(RunType::from_source(None), RunType::InteractiveChat);
    assert_eq!(RunType::from_source(Some("ptt")), RunType::InteractiveChat);
    assert_eq!(RunType::from_source(Some("type")), RunType::InteractiveChat);
    assert_eq!(
        RunType::from_source(Some("autonomous")),
        RunType::AutonomousTask
    );
    assert_eq!(
        RunType::from_source(Some("channel_inbound")),
        RunType::ChannelInbound
    );
    assert_eq!(RunType::AutonomousTask.as_str(), "autonomous_task");
}

#[test]
fn run_type_is_stamped_on_the_turn_span() {
    // Default: interactive chat.
    let mut c = SpanCollector::new(ctx());
    c.record(&AgentProgress::TurnStarted, 0);
    assert_eq!(
        find(c.spans(), "agent.turn").attributes["run.type"],
        serde_json::json!("interactive_chat")
    );

    // Explicit autonomous run.
    let mut c = SpanCollector::new(ctx().with_run_type(RunType::AutonomousTask));
    c.record(&AgentProgress::TurnStarted, 0);
    assert_eq!(
        find(c.spans(), "agent.turn").attributes["run.type"],
        serde_json::json!("autonomous_task")
    );
}

// ── error text capture (Langfuse statusMessage source) ─────────────────────

#[test]
fn subagent_failure_records_truncated_error_message_when_capture_on() {
    let mut c = SpanCollector::new(ctx().with_capture_content(true));
    c.record(&AgentProgress::TurnStarted, 0);
    c.record(&spawn("task-9", "Coder"), 5);
    let long_error = "boom ".repeat(200); // 1000 chars > 500 cap
    c.record(
        &AgentProgress::SubagentFailed {
            agent_id: "coder".to_string(),
            task_id: "task-9".to_string(),
            error: long_error,
        },
        10,
    );
    let sub = find(c.spans(), "subagent.Coder");
    assert_eq!(sub.status, SpanStatus::Error);
    let message = sub.attributes["error.message"].as_str().unwrap();
    assert!(message.starts_with("boom "));
    assert!(message.contains("[truncated"), "500-char cap must apply");
    assert!(message.chars().count() < 600);
}

#[test]
fn failed_tool_records_classified_cause_only_when_capture_on() {
    use crate::openhuman::tools::status::{ClassifiedFailure, FailureCategory, ToolFailureClass};
    let failed = AgentProgress::ToolCallCompleted {
        call_id: "c1".to_string(),
        tool_name: "shell".to_string(),
        success: false,
        output_chars: 0,
        output: String::new(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: Some(ClassifiedFailure {
            class: ToolFailureClass::Timeout,
            category: FailureCategory::Recoverable,
            cause_plain: "The command ran past its deadline".to_string(),
            next_action: "Try again".to_string(),
            recoverable: true,
        }),
    };

    // Capture ON → plain-language cause lands as error.message.
    let mut on = SpanCollector::new(ctx().with_capture_content(true));
    on.record(&AgentProgress::TurnStarted, 0);
    on.record(&tool_started("c1", "shell", 1), 1);
    on.record(&failed, 2);
    let tool = find(on.spans(), "tool.shell");
    assert_eq!(tool.status, SpanStatus::Error);
    assert_eq!(
        tool.attributes["error.message"],
        serde_json::json!("The command ran past its deadline")
    );

    // Capture OFF → no error text on the span.
    let mut off = SpanCollector::new(ctx());
    off.record(&AgentProgress::TurnStarted, 0);
    off.record(&tool_started("c1", "shell", 1), 1);
    off.record(&failed, 2);
    let tool = find(off.spans(), "tool.shell");
    assert_eq!(tool.status, SpanStatus::Error);
    assert!(tool.attributes.get("error.message").is_none());
}

#[test]
fn subagent_error_text_stays_out_without_capture() {
    // The pre-existing privacy behavior (length-only) holds with capture off —
    // covered by `subagent_failure_records_error_without_raw_text` above; here
    // we double-check error.message is absent.
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-9", "Coder"), 5),
        (
            AgentProgress::SubagentFailed {
                agent_id: "coder".to_string(),
                task_id: "task-9".to_string(),
                error: "secret path /Users/x".to_string(),
            },
            10,
        ),
    ]);
    c.finish(50);
    let sub = find(c.spans(), "subagent.Coder");
    assert!(sub.attributes.get("error.message").is_none());
}

#[test]
fn span_ids_are_unique_across_turns() {
    // Two separate collectors (two turns) must not reuse span ids, or Langfuse
    // dedupes their observations onto whichever trace claimed the id first.
    let a = collect(&[(AgentProgress::TurnStarted, 1)]);
    let b = collect(&[(AgentProgress::TurnStarted, 1)]);
    assert_ne!(
        find(a.spans(), "agent.turn").span_id,
        find(b.spans(), "agent.turn").span_id,
        "span ids must be globally unique across turns"
    );
}

#[test]
fn generation_records_request_messages_and_completion_when_capture_on() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (model_call_with_content(None), 10),
    ]);
    c.finish(20);
    let generation = find(c.spans(), "llm.chat-v1");
    let input = generation.input.as_ref().expect("generation input");
    assert!(
        input.to_string().contains("You are OpenHuman."),
        "system prompt must land in the generation input: {input}"
    );
    assert!(generation
        .output
        .as_ref()
        .expect("generation output")
        .to_string()
        .contains("hello"));
}

#[test]
fn generation_withholds_content_when_capture_off() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (model_call_with_content(None), 10),
    ]);
    c.finish(20);
    let generation = find(c.spans(), "llm.chat-v1");
    assert!(generation.input.is_none(), "capture off → no prompt");
    assert!(generation.output.is_none(), "capture off → no completion");
}

#[test]
fn subagent_model_call_nests_generation_and_stamps_model_on_subagent_span() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-9", "Context Scout"), 5),
        (
            AgentProgress::SubagentIterationStarted {
                agent_id: "context_scout".to_string(),
                task_id: "task-9".to_string(),
                iteration: 1,
                max_iterations: 8,
                extended_policy: true,
            },
            6,
        ),
        (model_call_with_content(Some("task-9")), 10),
    ]);
    c.finish(20);
    let spans = c.spans();

    // Generation nests under the subagent's iteration, not the parent turn.
    let generation = find(spans, "llm.chat-v1");
    let child_iter = find(spans, "subagent.iteration#1");
    assert_eq!(
        generation.parent_span_id.as_deref(),
        Some(child_iter.span_id.as_str())
    );
    assert!(generation.input.is_some(), "child generation carries input");

    // The subagent span itself surfaces the provider-labeled model + usage.
    let sub = find(spans, "subagent.Context Scout");
    assert_eq!(
        sub.attributes["gen_ai.request.model"],
        serde_json::json!("managed.chat-v1")
    );
    assert_eq!(
        sub.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100)
    );
    assert_eq!(
        sub.attributes["gen_ai.usage.cost_usd"],
        serde_json::json!(0.001)
    );

    // The parent turn's rollup is NOT polluted by the child call.
    let turn = find(spans, "agent.turn");
    assert!(turn.attributes.get("gen_ai.request.model").is_none());
}

#[test]
fn parent_tool_completion_backfills_arguments_and_records_output() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (
            // tinyagents path: Started carries Null arguments.
            AgentProgress::ToolCallStarted {
                call_id: "c1".to_string(),
                tool_name: "web_search".to_string(),
                arguments: serde_json::Value::Null,
                iteration: 1,
                display_label: None,
                display_detail: None,
            },
            5,
        ),
        (
            AgentProgress::ToolCallCompleted {
                call_id: "c1".to_string(),
                tool_name: "web_search".to_string(),
                success: true,
                output_chars: 7,
                output: "results".to_string(),
                arguments: Some(serde_json::json!({"query": "weather"})),
                elapsed_ms: 40,
                iteration: 1,
                failure: None,
            },
            45,
        ),
    ]);
    c.finish(50);
    let tool = find(c.spans(), "tool.web_search");
    assert!(
        tool.input
            .as_ref()
            .expect("tool input backfilled from completion")
            .to_string()
            .contains("weather"),
        "arguments from the completion event must backfill the span input"
    );
    assert_eq!(
        tool.output,
        Some(serde_json::Value::String("results".to_string()))
    );
}

#[test]
fn subagent_span_records_prompt_and_final_output_when_capture_on() {
    let mut c = collect_with_capture(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-1", "Researcher"), 5),
        (
            AgentProgress::SubagentCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                elapsed_ms: 100,
                iterations: 2,
                output_chars: 12,
                output: "final answer".to_string(),
                worktree_path: None,
                changed_files: vec![],
                dirty_status: None,
            },
            105,
        ),
    ]);
    c.finish(110);
    let sub = find(c.spans(), "subagent.Researcher");
    assert_eq!(
        sub.input,
        Some(serde_json::Value::String("delegated prompt".to_string()))
    );
    assert_eq!(
        sub.output,
        Some(serde_json::Value::String("final answer".to_string()))
    );
}
