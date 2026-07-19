use super::journal_projection::spans_from_observations;
use super::{SpanKind, SpanStatus, TraceContext};
use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::ids::{CallId, EventId, RunId};
use tinyagents::harness::observability::AgentObservation;
use tinyagents::harness::usage::Usage;

/// Wraps an event as a journalled observation stamped at `ts`.
fn obs(offset: u64, ts: u64, event: AgentEvent) -> AgentObservation {
    AgentObservation {
        event_id: EventId::new(format!("run-1-evt-{offset}")),
        run_id: RunId::new("run-1"),
        parent_run_id: None,
        root_run_id: RunId::new("run-1"),
        offset,
        ts_ms: ts,
        event,
    }
}

fn tool_completed(call: &str, name: &str, error: Option<&str>) -> AgentEvent {
    AgentEvent::ToolCompleted {
        call_id: CallId::new(call),
        tool_name: name.to_string(),
        started_at_ms: Some(1_020),
        input: None,
        output: None,
        duration_ms: Some(30),
        output_bytes: Some(12),
        error: error.map(str::to_string),
    }
}

fn single_turn(tool_error: Option<&str>) -> Vec<AgentObservation> {
    vec![
        obs(
            0,
            1_000,
            AgentEvent::RunStarted {
                run_id: RunId::new("run-1"),
                thread_id: None,
            },
        ),
        obs(
            1,
            1_010,
            AgentEvent::ModelStarted {
                call_id: CallId::new("c1"),
                model: "gpt-4".to_string(),
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::ToolStarted {
                call_id: CallId::new("t1"),
                tool_name: "lookup".to_string(),
            },
        ),
        obs(3, 1_050, tool_completed("t1", "lookup", tool_error)),
        obs(
            4,
            1_060,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("c1"),
                started_at_ms: Some(1_010),
                usage: Some(Usage::new(100, 20)),
                input: None,
                output: None,
            },
        ),
        obs(
            5,
            1_070,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ]
}

fn subagent_turn() -> Vec<AgentObservation> {
    vec![
        obs(
            0,
            1_000,
            AgentEvent::RunStarted {
                run_id: RunId::new("run-1"),
                thread_id: None,
            },
        ),
        obs(
            1,
            1_010,
            AgentEvent::SubAgentStarted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::ModelStarted {
                call_id: CallId::new("scout-model"),
                model: "gpt-4".to_string(),
            },
        ),
        obs(
            3,
            1_030,
            AgentEvent::ToolStarted {
                call_id: CallId::new("scout-tool"),
                tool_name: "read_file".to_string(),
            },
        ),
        obs(4, 1_060, tool_completed("scout-tool", "read_file", None)),
        obs(
            5,
            1_070,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("scout-model"),
                started_at_ms: Some(1_020),
                usage: Some(Usage::new(11, 7)),
                input: None,
                output: None,
            },
        ),
        obs(
            6,
            1_090,
            AgentEvent::SubAgentCompleted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            7,
            1_100,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ]
}

fn ctx() -> TraceContext {
    TraceContext::new("session-1", Some("user-1".into()))
}

#[test]
fn projects_single_agent_turn_span_tree() {
    let spans = spans_from_observations(ctx(), 10, &single_turn(None));

    // Turn + iteration + tool + generation spans are all present.
    let by_kind = |k: SpanKind| spans.iter().filter(|s| s.kind == k).count();
    assert_eq!(by_kind(SpanKind::Turn), 1, "one turn span");
    assert_eq!(by_kind(SpanKind::Iteration), 1, "one iteration span");
    assert_eq!(by_kind(SpanKind::Tool), 1, "one tool span");
    assert_eq!(by_kind(SpanKind::Generation), 1, "one generation span");

    let tool = spans.iter().find(|s| s.kind == SpanKind::Tool).unwrap();
    assert_eq!(tool.name, "tool.lookup");
    assert_eq!(tool.attributes["tool.success"], serde_json::json!(true));
    assert_eq!(tool.attributes["tool.output_chars"], serde_json::json!(12));
    assert_eq!(tool.attributes["tool.elapsed_ms"], serde_json::json!(30));

    let generation = spans
        .iter()
        .find(|s| s.kind == SpanKind::Generation)
        .unwrap();
    assert!(
        generation.name.contains("gpt-4"),
        "gen span names the model"
    );
}

#[test]
fn failed_tool_projects_error_outcome() {
    // With content capture on, a failed tool span carries the classified
    // cause reconstructed from the journalled error string, reproducing the
    // live path's `classify(error, false)` exactly.
    let spans = spans_from_observations(
        ctx().with_capture_content(true),
        10,
        &single_turn(Some("permission denied opening /etc/x")),
    );
    let tool = spans.iter().find(|s| s.kind == SpanKind::Tool).unwrap();
    assert_eq!(tool.attributes["tool.success"], serde_json::json!(false));
    assert!(
        tool.attributes.contains_key("error.message"),
        "failed tool span carries a classified error message"
    );
}

#[test]
fn projects_subagent_scope_from_lifecycle_brackets() {
    let spans = spans_from_observations(ctx(), 10, &subagent_turn());

    let by_kind = |k: SpanKind| spans.iter().filter(|s| s.kind == k).count();
    assert_eq!(by_kind(SpanKind::Subagent), 1, "one subagent span");
    assert_eq!(
        by_kind(SpanKind::SubagentIteration),
        1,
        "one child iteration span"
    );
    assert_eq!(by_kind(SpanKind::Tool), 1, "one child tool span");
    assert_eq!(by_kind(SpanKind::Generation), 1, "one child generation");

    let subagent = spans.iter().find(|s| s.kind == SpanKind::Subagent).unwrap();
    assert_eq!(
        subagent.attributes["subagent.agent_id"],
        serde_json::json!("researcher")
    );
    assert_eq!(
        subagent.attributes["subagent.iterations"],
        serde_json::json!(1)
    );

    let child_iteration = spans
        .iter()
        .find(|s| s.kind == SpanKind::SubagentIteration)
        .unwrap();
    assert_eq!(
        child_iteration.parent_span_id.as_deref(),
        Some(subagent.span_id.as_str())
    );
}

#[test]
fn projects_failed_subagent_from_child_run_failed() {
    let observations = vec![
        obs(
            0,
            1_000,
            AgentEvent::RunStarted {
                run_id: RunId::new("run-1"),
                thread_id: None,
            },
        ),
        obs(
            1,
            1_010,
            AgentEvent::SubAgentStarted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::RunFailed {
                run_id: RunId::new("run-1"),
                error: "provider unavailable".to_string(),
            },
        ),
        obs(
            3,
            1_030,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ];

    let spans = spans_from_observations(ctx(), 10, &observations);
    let subagent = spans.iter().find(|s| s.kind == SpanKind::Subagent).unwrap();

    assert_eq!(subagent.status, SpanStatus::Error);
    assert_eq!(subagent.attributes["error"], serde_json::json!(true));
    assert!(
        subagent.attributes.get("error.length").is_some(),
        "failed subagent span carries redacted error metadata"
    );
}

#[test]
fn projects_turn_content_from_root_model_io() {
    let observations = vec![
        obs(
            0,
            1_000,
            AgentEvent::RunStarted {
                run_id: RunId::new("run-1"),
                thread_id: None,
            },
        ),
        obs(
            1,
            1_010,
            AgentEvent::ModelStarted {
                call_id: CallId::new("m1"),
                model: "gpt-4".to_string(),
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("m1"),
                started_at_ms: Some(1_010),
                usage: Some(Usage::new(5, 3)),
                input: Some(serde_json::json!([
                    {"role": "user", "content": "summarize this"}
                ])),
                output: Some(serde_json::json!({
                    "role": "assistant",
                    "content": "short summary"
                })),
            },
        ),
        obs(
            3,
            1_030,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ];
    let spans = spans_from_observations(ctx().with_capture_content(true), 10, &observations);
    let turn = spans.iter().find(|s| s.kind == SpanKind::Turn).unwrap();

    assert!(
        turn.input
            .as_ref()
            .unwrap()
            .to_string()
            .contains("summarize this"),
        "root model input is attached through TurnContent"
    );
    assert_eq!(
        turn.output.as_ref().unwrap(),
        &serde_json::json!("short summary")
    );
}
