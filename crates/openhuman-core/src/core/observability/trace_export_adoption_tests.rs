use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::ids::{CallId, EventId, RunId};
use tinyagents_harness::observability::trace_export::collector::SpanCollector as NativeSpanCollector;
use tinyagents_harness::observability::trace_export::serialize::{spans_to_ndjson, SpanEnvelope};
use tinyagents_harness::observability::trace_export::TraceContext;
use tinyagents_harness::observability::trace_export::TraceSpan;
use tinyagents_harness::observability::AgentObservation;
use tinyinference_llm::usage::Usage;

fn root_observation(offset: u64, ts_ms: u64, event: AgentEvent) -> AgentObservation {
    AgentObservation {
        event_id: EventId::new(format!("root-evt-{offset}")),
        run_id: RunId::new("root-run"),
        parent_run_id: None,
        root_run_id: RunId::new("root-run"),
        offset,
        ts_ms,
        event,
    }
}

fn child_observation(offset: u64, ts_ms: u64, event: AgentEvent) -> AgentObservation {
    AgentObservation {
        event_id: EventId::new(format!("child-evt-{offset}")),
        run_id: RunId::new("child-run"),
        parent_run_id: Some(RunId::new("root-run")),
        root_run_id: RunId::new("root-run"),
        offset,
        ts_ms,
        event,
    }
}

fn parent_run() -> Vec<AgentObservation> {
    vec![
        root_observation(
            0,
            1_000,
            AgentEvent::RunStarted {
                run_id: RunId::new("root-run"),
                thread_id: None,
            },
        ),
        root_observation(1, 1_005, AgentEvent::TurnStarted { turn: 1 }),
        root_observation(
            2,
            1_010,
            AgentEvent::ModelStarted {
                call_id: CallId::new("same-call"),
                model: "gpt-4".to_string(),
            },
        ),
        root_observation(
            3,
            1_020,
            AgentEvent::ToolStarted {
                parent_call_id: None,
                call_id: CallId::new("tool-1"),
                tool_name: "lookup".to_string(),
                input: Some(serde_json::json!({"query":"CONTENT_SENTINEL_INPUT"})),
            },
        ),
        root_observation(
            4,
            1_030,
            AgentEvent::SubAgentRunStarted {
                name: "researcher".to_string(),
                depth: 1,
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            5,
            1_080,
            AgentEvent::SubAgentRunCompleted {
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            6,
            1_090,
            AgentEvent::ToolCompleted {
                parent_call_id: None,
                call_id: CallId::new("tool-1"),
                tool_name: "lookup".to_string(),
                started_at_ms: Some(1_020),
                input: Some(serde_json::json!({"query":"CONTENT_SENTINEL_INPUT"})),
                output: Some(serde_json::json!({"result":"CONTENT_SENTINEL_OUTPUT"})),
                duration_ms: Some(70),
                input_bytes: None,
                output_bytes: Some(33),
                error: None,
                metadata: None,
            },
        ),
        root_observation(
            7,
            1_100,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("same-call"),
                started_at_ms: Some(1_010),
                usage: Some(Usage::new(100, 20)),
                input: Some(serde_json::json!(["CONTENT_SENTINEL_PROMPT"])),
                output: Some(serde_json::json!(["CONTENT_SENTINEL_REPLY"])),
            },
        ),
        root_observation(
            8,
            1_105,
            AgentEvent::TurnCompleted {
                turn: 1,
                tool_result_count: 1,
                tool_call_ids: vec![CallId::new("tool-1")],
            },
        ),
        root_observation(
            9,
            1_110,
            AgentEvent::RunCompleted {
                run_id: RunId::new("root-run"),
                outcome: None,
            },
        ),
    ]
}

fn child_run() -> Vec<AgentObservation> {
    vec![
        child_observation(
            0,
            1_040,
            AgentEvent::RunStarted {
                run_id: RunId::new("child-run"),
                thread_id: None,
            },
        ),
        child_observation(1, 1_045, AgentEvent::TurnStarted { turn: 1 }),
        child_observation(
            2,
            1_050,
            AgentEvent::ModelStarted {
                call_id: CallId::new("same-call"),
                model: "gpt-4-mini".to_string(),
            },
        ),
        child_observation(
            3,
            1_060,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("same-call"),
                started_at_ms: Some(1_050),
                usage: Some(Usage::new(7, 3)),
                input: None,
                output: None,
            },
        ),
        child_observation(
            4,
            1_065,
            AgentEvent::TurnCompleted {
                turn: 1,
                tool_result_count: 0,
                tool_call_ids: Vec::new(),
            },
        ),
        child_observation(
            5,
            1_070,
            AgentEvent::RunCompleted {
                run_id: RunId::new("child-run"),
                outcome: None,
            },
        ),
    ]
}

fn normalized(spans: &[TraceSpan]) -> Vec<serde_json::Value> {
    let spans_by_id: std::collections::HashMap<_, _> = spans
        .iter()
        .map(|span| (span.span_id.as_str(), span))
        .collect();
    spans
        .iter()
        .map(|span| {
            serde_json::json!({
                "name": span.name,
                "kind": format!("{:?}", span.kind),
                "parent": span.parent_span_id.as_deref().and_then(|id| spans_by_id.get(id)).map(|parent| parent.name.as_str()),
                "start_unix_ms": span.start_unix_ms,
                "end_unix_ms": span.end_unix_ms,
                "status": format!("{:?}", span.status),
                "attributes": span.attributes,
            })
        })
        .collect()
}

#[test]
fn per_run_trace_snapshots_keep_reused_call_ids_scoped_and_content_gated() {
    let parent_ctx = TraceContext::new("trace-parent", Some("user-1".to_string()))
        .with_run_lineage(Some("root-run".into()), None, Some("root-run".into()));
    let parent_observations = parent_run();
    let child_observations = child_run();
    let observations = crate::agent::tinyagents::journal::merge_run_observations(vec![
        parent_observations.clone(),
        child_observations.clone(),
    ]);
    let mut native = NativeSpanCollector::new(parent_ctx.clone());
    for observation in &observations {
        native.record(observation);
    }
    native.finish(1_110);
    let snapshot = normalized(native.spans());
    println!(
        "NATIVE_TREE:{}",
        serde_json::to_string_pretty(&snapshot).unwrap()
    );

    assert!(
        !spans_to_ndjson(SpanEnvelope::Otel, native.spans()).contains("CONTENT_SENTINEL"),
        "content-off export must not contain parent or child payloads"
    );
    let mut captured = NativeSpanCollector::new(parent_ctx.with_capture_content(true));
    for observation in &observations {
        captured.record(observation);
    }
    captured.finish(1_110);
    assert!(
        spans_to_ndjson(SpanEnvelope::Otel, captured.spans()).contains("CONTENT_SENTINEL_PROMPT"),
        "content-on export retains opted-in prompt content"
    );
    let root_generation = native
        .spans()
        .iter()
        .find(|span| span.name == "llm.gpt-4")
        .expect("root generation survives export");
    let child_generation = native
        .spans()
        .iter()
        .find(|span| span.name == "llm.gpt-4-mini")
        .expect("child generation survives export despite reused call id");
    assert_eq!(
        root_generation.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100)
    );
    assert_eq!(
        child_generation.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(7)
    );
    assert!(snapshot
        .iter()
        .any(|span| span["name"] == "agent.iteration#1"));
    assert!(snapshot
        .iter()
        .any(|span| span["name"] == "subagent.iteration#1"));
}

#[test]
fn child_native_failure_survives_parent_acknowledgement_at_same_timestamp() {
    use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};
    use tinyagents_harness::terminal::{TerminalOutcome, TerminalReason};

    let parent = vec![
        root_observation(
            0,
            10,
            AgentEvent::RunStarted {
                run_id: RunId::new("root-run"),
                thread_id: None,
            },
        ),
        root_observation(
            1,
            11,
            AgentEvent::SubAgentRunStarted {
                name: "researcher".into(),
                depth: 1,
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            2,
            20,
            AgentEvent::SubAgentRunCompleted {
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            3,
            30,
            AgentEvent::RunCompleted {
                run_id: RunId::new("root-run"),
                outcome: Some(TerminalOutcome::new(TerminalReason::Completed, "done")),
            },
        ),
    ];
    let child = vec![
        child_observation(
            0,
            12,
            AgentEvent::RunStarted {
                run_id: RunId::new("child-run"),
                thread_id: None,
            },
        ),
        child_observation(
            1,
            20,
            AgentEvent::RunFailed {
                run_id: RunId::new("child-run"),
                error: "child provider failed".into(),
                outcome: Some(TerminalOutcome::new(
                    TerminalReason::ToolFailed,
                    "child provider failed",
                )),
            },
        ),
    ];
    let observations =
        crate::agent::tinyagents::journal::merge_run_observations(vec![parent, child]);
    assert!(matches!(
        observations[3].event,
        AgentEvent::SubAgentRunCompleted { .. }
    ));
    assert!(matches!(
        observations[4].event,
        AgentEvent::RunFailed { .. }
    ));

    let mut collector =
        NativeSpanCollector::new(TraceContext::new("root-trace", None).with_run_lineage(
            Some("root-run".into()),
            None,
            Some("root-run".into()),
        ));
    for observation in &observations {
        collector.record(observation);
    }
    collector.finish(30);
    let child_span = collector
        .spans()
        .iter()
        .find(|span| span.kind == SpanKind::Subagent)
        .expect("native child span");
    assert_eq!(child_span.status, SpanStatus::Error);
    assert_eq!(
        child_span.attributes["error.message"], "Operation failed",
        "terminal failures use the content-free native fallback, not raw provider prose"
    );
}

#[test]
fn run_finalization_bounds_orphaned_child_tools_before_export_finish() {
    use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};
    use tinyagents_harness::terminal::{TerminalOutcome, TerminalReason};

    let parent = vec![
        root_observation(
            0,
            10,
            AgentEvent::RunStarted {
                run_id: RunId::new("root-run"),
                thread_id: None,
            },
        ),
        root_observation(
            1,
            11,
            AgentEvent::SubAgentRunStarted {
                name: "researcher".into(),
                depth: 1,
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            2,
            20,
            AgentEvent::RunCompleted {
                run_id: RunId::new("root-run"),
                outcome: Some(TerminalOutcome::new(TerminalReason::Completed, "done")),
            },
        ),
        root_observation(
            3,
            29,
            AgentEvent::SubAgentRunCompleted {
                run_id: RunId::new("child-run"),
            },
        ),
        root_observation(
            4,
            30,
            AgentEvent::RunFinalized {
                run_id: RunId::new("root-run"),
            },
        ),
    ];
    let child = vec![
        child_observation(
            0,
            12,
            AgentEvent::RunStarted {
                run_id: RunId::new("child-run"),
                thread_id: None,
            },
        ),
        child_observation(
            1,
            15,
            AgentEvent::ToolStarted {
                parent_call_id: None,
                call_id: CallId::new("orphan-tool-call"),
                tool_name: "lookup".into(),
                input: None,
            },
        ),
        child_observation(
            2,
            18,
            AgentEvent::RunCompleted {
                run_id: RunId::new("child-run"),
                outcome: Some(TerminalOutcome::new(TerminalReason::Completed, "done")),
            },
        ),
        child_observation(
            3,
            28,
            AgentEvent::RunFinalized {
                run_id: RunId::new("child-run"),
            },
        ),
    ];

    // This is the same composition used by export_run_trace_from_journal:
    // each run keeps its journal offset order, while finalization timestamps
    // arrive after the harness terminal event and the host persists the result.
    let observations =
        crate::agent::tinyagents::journal::merge_run_observations(vec![parent, child]);
    let mut collector =
        NativeSpanCollector::new(TraceContext::new("root-trace", None).with_run_lineage(
            Some("root-run".into()),
            None,
            Some("root-run".into()),
        ));
    for observation in &observations {
        collector.record(observation);
    }
    collector.finish(1_000);

    let root = collector
        .spans()
        .iter()
        .find(|span| span.kind == SpanKind::Turn)
        .expect("root run span");
    assert_eq!(root.end_unix_ms, Some(30));
    let orphan = collector
        .spans()
        .iter()
        .find(|span| span.name == "tool.lookup")
        .expect("orphan child tool span");
    assert_eq!(orphan.end_unix_ms, Some(28));
    assert_eq!(orphan.status, SpanStatus::Unset);
    assert_eq!(orphan.attributes["force_closed"], true);
    assert_eq!(orphan.attributes["observation.level"], "WARNING");
    let child = collector
        .spans()
        .iter()
        .find(|span| span.kind == SpanKind::Subagent)
        .expect("linked child span");
    assert_eq!(child.end_unix_ms, Some(29));
}

#[test]
fn separate_child_trace_roots_observations_but_retains_parent_lineage() {
    use tinyagents_harness::observability::trace_export::{RunType, SpanKind};

    let child = child_run();
    let rooted =
        tinyagents_harness::observability::trace_export::journal_export::root_subagent_observations(
            &child,
        );
    let trace_context = TraceContext::new("subagent:child-run", Some("user-1".into()))
        .with_session_group("conversation-1")
        .with_agent_id("researcher")
        .with_channel_source("subagent")
        .with_run_type(RunType::Subagent)
        .with_capture_content(false)
        .with_run_lineage(
            Some("child-run".into()),
            Some("root-run".into()),
            Some("root-run".into()),
        );
    let mut collector =
        NativeSpanCollector::new(trace_context).with_observation_root(RunId::new("child-run"));
    for observation in &rooted {
        collector.record(observation);
    }
    collector.finish(1_070);

    let root = collector
        .spans()
        .iter()
        .find(|span| span.kind == SpanKind::Turn)
        .expect("child trace has its own root turn");
    assert_eq!(root.attributes["run.id"], "child-run");
    assert_eq!(root.attributes["parent_run_id"], "root-run");
    assert_eq!(root.attributes["root_run_id"], "root-run");
    assert!(collector
        .spans()
        .iter()
        .all(|span| span.trace_id == "subagent:child-run"));
}
