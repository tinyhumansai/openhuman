//! Turns and sub-agents the harness stopped early (failure breaker, deadline
//! wind-down, iteration cap) reach the normal completion path. Their spans
//! must still say so: `WARNING` level, a content-free status message and
//! `turn.stop_*` attributes, so a level filter finds them.

use super::*;

use tinyagents_harness::observability::trace_export::otlp::otlp_requests;
use tinyagents_harness::observability::trace_export::SpanStatus;
use tinyagents_harness::turn_stop::TurnStop;

const BREAKER_NOTE: &str = "Stopping after 2 attempt(s): failure class \
    `uncertain_side_effect` still blocks operation `web_answer_tool` on \
    `https://example.com/a`. Resolve this blocker before retrying.";

fn attr<'a>(span: &'a TraceSpan, key: &str) -> Option<&'a str> {
    span.attributes.get(key).and_then(|value| value.as_str())
}

fn root(spans: &[TraceSpan]) -> &TraceSpan {
    spans
        .iter()
        .find(|span| span.parent_span_id.is_none())
        .expect("turn span")
}

fn exported_level(spans: &[TraceSpan], name: &str) -> Option<String> {
    let payloads = otlp_requests(
        spans,
        "production",
        &crate::agent::progress_tracing::export_brand(),
    );
    payloads
        .iter()
        .flat_map(|payload| {
            payload["resourceSpans"][0]["scopeSpans"][0]["spans"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .find(|span| span["name"] == name)
        .and_then(|span| {
            span["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["key"] == "langfuse.observation.level")
                .and_then(|item| item["value"]["stringValue"].as_str())
                .map(str::to_string)
        })
}

#[test]
fn a_breaker_stopped_turn_closes_at_warning_with_stop_attributes() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::TurnCompleted {
                iterations: 4,
                stop: Some(crate::agent::turn_projection::breaker_stop(BREAKER_NOTE)),
            },
            50,
        ),
    ]);
    c.finish(50);
    let spans = c.into_spans();
    let turn = root(&spans);
    assert_ne!(
        turn.status,
        SpanStatus::Error,
        "a stop is not a harness error"
    );
    assert_ne!(
        turn.status,
        SpanStatus::Ok,
        "a stop is not a clean completion"
    );
    assert_eq!(attr(turn, "observation.level"), Some("WARNING"));
    assert_eq!(attr(turn, "turn.outcome"), Some("stopped"));
    assert_eq!(attr(turn, "turn.stop_kind"), Some("breaker"));
    assert_eq!(attr(turn, "turn.stop_class"), Some("uncertain_side_effect"));
    assert_eq!(attr(turn, "turn.stop_operation"), Some("web_answer_tool"));
    let message = "stopped: breaker uncertain_side_effect on web_answer_tool";
    assert_eq!(attr(turn, "observation.status_message"), Some(message));
    // The OTLP converter reads the Langfuse statusMessage from `error.message`.
    assert_eq!(attr(turn, "error.message"), Some(message));
    assert_eq!(
        turn.attributes.get("agent.iterations"),
        Some(&serde_json::json!(4))
    );
    assert_eq!(
        exported_level(&spans, &turn.name).as_deref(),
        Some("WARNING"),
        "the OTLP export must carry the WARNING level"
    );
}

#[test]
fn a_wind_down_or_capped_turn_is_stopped_too() {
    for (stop, kind) in [
        (TurnStop::wind_down(), "wind_down"),
        (TurnStop::iteration_cap(), "iteration_cap"),
    ] {
        let mut c = collect(&[
            (AgentProgress::TurnStarted, 0),
            (
                AgentProgress::TurnCompleted {
                    iterations: 2,
                    stop: Some(stop),
                },
                10,
            ),
        ]);
        c.finish(10);
        let spans = c.into_spans();
        let turn = root(&spans);
        assert_eq!(attr(turn, "observation.level"), Some("WARNING"), "{kind}");
        assert_eq!(attr(turn, "turn.stop_kind"), Some(kind));
        assert_eq!(attr(turn, "turn.stop_class"), None);
        assert_eq!(
            attr(turn, "observation.status_message").map(str::to_string),
            Some(format!("stopped: {kind}"))
        );
    }
}

#[test]
fn a_turn_that_finished_on_its_own_stays_completed() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (
            AgentProgress::TurnCompleted {
                iterations: 1,
                stop: None,
            },
            10,
        ),
    ]);
    c.finish(10);
    let spans = c.into_spans();
    let turn = root(&spans);
    assert_eq!(turn.status, SpanStatus::Ok);
    assert_eq!(attr(turn, "turn.outcome"), Some("completed"));
    assert_eq!(attr(turn, "observation.level"), None);
    assert_eq!(attr(turn, "turn.stop_kind"), None);
}

fn subagent_completed(task: &str, stop: Option<TurnStop>) -> AgentProgress {
    AgentProgress::SubagentCompleted {
        agent_id: "researcher".to_string(),
        task_id: task.to_string(),
        elapsed_ms: 30,
        iterations: 3,
        output_chars: 10,
        output: "partial".to_string(),
        usage: None,
        worktree_path: None,
        changed_files: Vec::new(),
        dirty_status: None,
        stop,
    }
}

#[test]
fn a_stopped_subagent_span_closes_at_warning_with_stop_attributes() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("t-stop", "Research"), 5),
        (
            subagent_completed(
                "t-stop",
                Some(crate::agent::turn_projection::breaker_stop(BREAKER_NOTE)),
            ),
            40,
        ),
    ]);
    c.finish(50);
    let spans = c.into_spans();
    let sub = find(&spans, "subagent.Research");
    assert_ne!(
        sub.status,
        SpanStatus::Ok,
        "a stopped child is not a clean finish"
    );
    assert_eq!(attr(sub, "observation.level"), Some("WARNING"));
    assert_eq!(attr(sub, "turn.outcome"), Some("stopped"));
    assert_eq!(attr(sub, "turn.stop_kind"), Some("breaker"));
    assert_eq!(attr(sub, "turn.stop_class"), Some("uncertain_side_effect"));
    assert_eq!(attr(sub, "turn.stop_operation"), Some("web_answer_tool"));
    assert_eq!(
        attr(sub, "observation.status_message"),
        Some("stopped: breaker uncertain_side_effect on web_answer_tool")
    );
    assert_eq!(
        exported_level(&spans, "subagent.Research").as_deref(),
        Some("WARNING")
    );
}

#[test]
fn a_subagent_that_finished_normally_stays_ok() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("t-ok", "Research"), 5),
        (subagent_completed("t-ok", None), 40),
    ]);
    c.finish(50);
    let spans = c.into_spans();
    let sub = find(&spans, "subagent.Research");
    assert_eq!(sub.status, SpanStatus::Ok);
    assert_eq!(attr(sub, "observation.level"), None);
    assert_eq!(attr(sub, "turn.stop_kind"), None);
}
