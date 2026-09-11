use super::*;

#[test]
fn subagent_content_is_withheld_when_capture_off() {
    let mut c = collect(&[
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
    assert!(sub.input.is_none());
    assert!(sub.output.is_none());
}

#[test]
fn oversized_model_content_degrades_to_truncated_string() {
    let big = "x".repeat(MAX_MODEL_CONTENT_CHARS + 100);
    let captured = capture_model_content(&serde_json::json!({ "content": big }));
    let rendered = match &captured {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    assert!(rendered.chars().count() <= MAX_MODEL_CONTENT_CHARS + 64);
    assert!(rendered.contains("truncated"));
}

#[test]
fn turn_content_respects_the_trace_context_capture_gate() {
    // Regression (PR #4506 review): the collector briefly carried TWO capture
    // gates — a collector-level flag (checked by the TurnContent arm) and the
    // TraceContext flag (checked everywhere else). The web progress bridge only
    // sets the TraceContext flag, so TurnContent silently dropped the turn's
    // prompt/reply even with capture_content enabled. There is now a single
    // gate: both construction styles must attach TurnContent.
    for collector in [
        SpanCollector::new(ctx().with_capture_content(true)),
        SpanCollector::new(ctx()).with_content_capture(true),
    ] {
        let mut c = collector;
        c.record(&AgentProgress::TurnStarted, 0);
        c.record(
            &AgentProgress::TurnContent {
                input: Some("the prompt".to_string()),
                output: Some("the reply".to_string()),
            },
            5,
        );
        c.finish(10);
        let turn = find(c.spans(), "agent.turn");
        assert_eq!(
            turn.input,
            Some(serde_json::Value::String("the prompt".to_string()))
        );
        assert_eq!(
            turn.output,
            Some(serde_json::Value::String("the reply".to_string()))
        );
    }
}
