//! Journal-projection tests for the turn cost roll-up, the recovered
//! unknown-tool call, and the exhaustive no-op arms (openhuman#6148).

use super::*;
use crate::openhuman::agent::progress_tracing::TraceSpan;

/// A top-level model call that journals usage: `ModelStarted` → `UsageRecorded`
/// → `ModelCompleted`, the order the agent loop emits them in
/// (`run_loop.rs` emits `UsageRecorded` before `ModelCompleted`).
fn turn_with_usage(usages: &[Usage]) -> Vec<AgentObservation> {
    let mut events = vec![obs(
        0,
        1_000,
        AgentEvent::RunStarted {
            run_id: RunId::new("run-1"),
            thread_id: None,
        },
    )];
    let mut offset = 1;
    for (index, usage) in usages.iter().enumerate() {
        let call = format!("c{}", index + 1);
        events.push(obs(
            offset,
            1_010 + offset * 10,
            AgentEvent::ModelStarted {
                call_id: CallId::new(&call),
                model: "gpt-4".to_string(),
            },
        ));
        offset += 1;
        events.push(obs(
            offset,
            1_010 + offset * 10,
            AgentEvent::UsageRecorded { usage: *usage },
        ));
        offset += 1;
        events.push(obs(
            offset,
            1_010 + offset * 10,
            AgentEvent::ModelCompleted {
                call_id: CallId::new(&call),
                started_at_ms: Some(1_010),
                usage: Some(*usage),
                input: None,
                output: None,
            },
        ));
        offset += 1;
    }
    events.push(obs(
        offset,
        1_010 + offset * 10,
        AgentEvent::RunCompleted {
            run_id: RunId::new("run-1"),
        },
    ));
    events
}

fn turn_span(spans: &[TraceSpan]) -> &TraceSpan {
    spans
        .iter()
        .find(|s| s.kind == SpanKind::Turn)
        .expect("a root turn span")
}

/// #6148: the journal-shadow parity check compares attribute *keys*, and the
/// root turn span's `gen_ai.*` group is written only by `TurnCostUpdated` —
/// which the projection never emitted, so every turn diverged.
#[test]
fn usage_recorded_projects_the_turn_cost_rollup() {
    let spans = spans_from_observations(ctx(), 10, &turn_with_usage(&[Usage::new(100, 20)]));
    let turn = turn_span(&spans);

    for key in [
        "gen_ai.request.model",
        "gen_ai.usage.input_tokens",
        "gen_ai.usage.output_tokens",
        "gen_ai.usage.cached_input_tokens",
        "gen_ai.usage.cost_usd",
    ] {
        assert!(
            turn.attributes.contains_key(key),
            "projected turn span carries {key} (parity with the live span)"
        );
    }
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.output_tokens"],
        serde_json::json!(20)
    );
    // `gen_ai.request.model` is deliberately NOT value-asserted here: the roll-up
    // writes the raw handle, and the `ModelCallCompleted` that follows overwrites
    // the root with the provider-labeled `{provider_id}.{model}` form on both
    // paths. The journal carries no `provider_id` (§2a), so this is `".gpt-4"`
    // here against `"openai.gpt-4"` live — a *value* difference the key-only
    // parity signature does not see, and the reason that key was never among the
    // four the shadow check reported missing.
}

/// The roll-up is cumulative across a turn's model calls, matching the live
/// bridge's running `BridgeState` totals rather than reporting the last call.
#[test]
fn turn_cost_rollup_accumulates_across_model_calls() {
    let spans = spans_from_observations(
        ctx(),
        10,
        &turn_with_usage(&[Usage::new(100, 20), Usage::new(50, 5)]),
    );
    let turn = turn_span(&spans);
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(150)
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.output_tokens"],
        serde_json::json!(25)
    );
}

/// The observe-only crate `BudgetMiddleware` re-emits `UsageRecorded` for every
/// model call, so the journal holds TWO identical events per call with distinct
/// event ids. The live bridge dedupes on the iteration cursor; without the same
/// guard here every projected total would be doubled.
#[test]
fn duplicate_usage_recorded_for_one_call_is_folded_once() {
    let mut observations = turn_with_usage(&[Usage::new(100, 20)]);
    // Re-emit the middleware's duplicate immediately after the loop's own event,
    // with a distinct offset (and therefore a distinct event id).
    observations.insert(
        3,
        obs(
            99,
            1_035,
            AgentEvent::UsageRecorded {
                usage: Usage::new(100, 20),
            },
        ),
    );

    let spans = spans_from_observations(ctx(), 10, &observations);
    let turn = turn_span(&spans);
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100),
        "the middleware's duplicate must not double the roll-up"
    );
    assert_eq!(
        turn.attributes["gen_ai.usage.output_tokens"],
        serde_json::json!(20)
    );
}

/// Live, a child run has its own bridge instance and its own accumulator, and
/// the per-child `TurnCostUpdated` is suppressed. The journal interleaves both
/// runs into one stream, so the projection must skip child usage outright or the
/// parent turn would over-report.
#[test]
fn subagent_usage_does_not_roll_into_the_parent_turn() {
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
                call_id: CallId::new("c1"),
                model: "gpt-4".to_string(),
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::UsageRecorded {
                usage: Usage::new(100, 20),
            },
        ),
        obs(
            3,
            1_030,
            AgentEvent::SubAgentStarted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            4,
            1_040,
            AgentEvent::ModelStarted {
                call_id: CallId::new("c2"),
                model: "gpt-4-mini".to_string(),
            },
        ),
        obs(
            5,
            1_050,
            AgentEvent::UsageRecorded {
                usage: Usage::new(7_000, 900),
            },
        ),
        obs(
            6,
            1_060,
            AgentEvent::SubAgentCompleted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            7,
            1_070,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ];

    let spans = spans_from_observations(ctx(), 10, &observations);
    let turn = turn_span(&spans);
    assert_eq!(
        turn.attributes["gen_ai.usage.input_tokens"],
        serde_json::json!(100),
        "the child's 7000 input tokens stay out of the parent roll-up"
    );
    assert_eq!(
        turn.attributes["gen_ai.request.model"],
        serde_json::json!("gpt-4"),
        "the child's model must not rename the parent turn"
    );
}

/// `journal_projection` used to hardcode `cache_creation_tokens: 0` even though
/// the crate `Usage` carries the real value. `record_model_call` inserts that
/// attribute only when `> 0`, so the zero dropped the key from every projected
/// generation span whenever a provider reported a cache write.
#[test]
fn cache_creation_tokens_reach_the_projected_generation_span() {
    let usage = Usage {
        cache_creation_tokens: 512,
        ..Usage::new(100, 20)
    };
    let spans = spans_from_observations(ctx(), 10, &turn_with_usage(&[usage]));
    let generation = spans
        .iter()
        .find(|s| s.kind == SpanKind::Generation)
        .expect("a generation span");
    assert_eq!(
        generation.attributes["gen_ai.usage.cache_creation_tokens"],
        serde_json::json!(512)
    );
}

/// #4118: the crate recovers an unavailable tool call without emitting
/// `ToolStarted`/`ToolCompleted`, and the live bridge synthesises the pair. The
/// projection had no arm, so it was short a whole tool span — a span *count*
/// divergence, not merely a missing attribute.
#[test]
fn unknown_tool_call_projects_a_failed_tool_span() {
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
                call_id: CallId::new("c1"),
                model: "gpt-4".to_string(),
            },
        ),
        obs(
            2,
            1_020,
            AgentEvent::UnknownToolCall {
                call_id: CallId::new("t1"),
                requested_name: "send_fax".to_string(),
                arguments: serde_json::json!({ "to": "1234" }),
                recovery: "rewrite:none".to_string(),
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
    let tool = spans
        .iter()
        .find(|s| s.kind == SpanKind::Tool)
        .expect("the recovered call still produces a tool span");
    assert_eq!(tool.name, "tool.send_fax");
    assert_eq!(tool.attributes["tool.success"], serde_json::json!(false));
    assert_eq!(tool.status, SpanStatus::Error);
}

/// The child-scope half of the `UnknownToolCall` projection: a sub-agent that
/// names an unavailable tool gets the same synthesised failed-call pair, nested
/// under its subagent span rather than the root turn.
#[test]
fn unknown_tool_call_inside_a_subagent_projects_a_child_tool_span() {
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
            AgentEvent::ModelStarted {
                call_id: CallId::new("c1"),
                model: "gpt-4-mini".to_string(),
            },
        ),
        obs(
            3,
            1_030,
            AgentEvent::UnknownToolCall {
                call_id: CallId::new("t1"),
                requested_name: "send_fax".to_string(),
                arguments: serde_json::json!({ "to": "1234" }),
                recovery: "rewrite:none".to_string(),
            },
        ),
        obs(
            4,
            1_040,
            AgentEvent::SubAgentCompleted {
                name: "researcher".to_string(),
                depth: 1,
            },
        ),
        obs(
            5,
            1_050,
            AgentEvent::RunCompleted {
                run_id: RunId::new("run-1"),
            },
        ),
    ];

    let spans = spans_from_observations(ctx(), 10, &observations);
    let subagent = spans
        .iter()
        .find(|s| s.kind == SpanKind::Subagent)
        .expect("a subagent span");
    let tool = spans
        .iter()
        .find(|s| s.kind == SpanKind::Tool)
        .expect("the recovered child call still produces a tool span");
    assert_eq!(tool.name, "tool.send_fax");
    assert_eq!(tool.attributes["tool.success"], serde_json::json!(false));
    assert_eq!(tool.status, SpanStatus::Error);
    // Assert the exact chain, not merely that a parent exists: the tool hangs
    // off the child's *iteration* span (`SubagentToolCallStarted` prefers the
    // sub-agent's `current_iteration_span_id`), which in turn hangs off the
    // sub-agent span. A presence check would pass even if the tool attached to
    // an unrelated iteration.
    let child_iteration = spans
        .iter()
        .find(|s| s.kind == SpanKind::SubagentIteration)
        .expect("the child iteration span brackets the recovered call");
    assert_eq!(
        child_iteration.parent_span_id.as_deref(),
        Some(subagent.span_id.as_str()),
        "child iteration hangs off the subagent span"
    );
    assert_eq!(
        tool.parent_span_id.as_deref(),
        Some(child_iteration.span_id.as_str()),
        "the recovered tool span hangs off the child iteration"
    );
}

/// The exhaustive no-op arms: events that carry no span data must project to
/// nothing, leaving the span tree byte-identical to the same journal without
/// them. Guards against a future arm being moved out of the no-op group by
/// accident.
#[test]
fn non_span_bearing_events_project_to_no_spans() {
    let baseline = spans_from_observations(ctx(), 10, &turn_with_usage(&[Usage::new(100, 20)]));

    let mut noisy = turn_with_usage(&[Usage::new(100, 20)]);
    noisy.insert(
        2,
        obs(
            90,
            1_015,
            AgentEvent::CacheHit {
                call_id: CallId::new("c1"),
                key: "k".to_string(),
            },
        ),
    );
    noisy.insert(3, obs(91, 1_016, AgentEvent::MemoryLoaded));
    noisy.insert(4, obs(92, 1_017, AgentEvent::StreamClosed));
    let projected = spans_from_observations(ctx(), 10, &noisy);

    assert_eq!(
        projected.len(),
        baseline.len(),
        "diagnostic events add no spans"
    );
    // Counts and kinds alone would still pass if a diagnostic event changed an
    // attribute, a name, a status or a parent link, so compare every stable
    // field. `trace_id`/`span_id` are generated per projection and are excluded;
    // the parent link is compared as an *index* into the span list, which pins
    // the tree shape without depending on generated ids.
    assert_eq!(span_shape(&projected), span_shape(&baseline));
}

/// Reduces spans to the fields that must be identical across two projections of
/// the same run — everything but the generated `trace_id`/`span_id`, with the
/// parent expressed positionally.
fn span_shape(spans: &[TraceSpan]) -> Vec<String> {
    spans
        .iter()
        .map(|span| {
            let parent = span
                .parent_span_id
                .as_ref()
                .and_then(|id| spans.iter().position(|other| &other.span_id == id));
            format!(
                "{:?}|{}|{:?}|parent={parent:?}|start={}|end={:?}|attrs={:?}|in={:?}|out={:?}",
                span.kind,
                span.name,
                span.status,
                span.start_unix_ms,
                span.end_unix_ms,
                span.attributes,
                span.input,
                span.output,
            )
        })
        .collect()
}
