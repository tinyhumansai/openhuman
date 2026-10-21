use super::*;

#[test]
fn wait_schema_requires_message() {
    let schema = WaitTool::new().parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required list");
    assert!(required.iter().any(|v| v.as_str() == Some("message")));
    assert!(schema["properties"].get("duration_secs").is_some());
    assert!(schema["properties"].get("duration_ms").is_some());
}

#[test]
fn wait_loop_schema_includes_loop_controls() {
    let schema = WaitLoopTool::new().parameters_schema();
    assert!(schema["properties"].get("loop_key").is_some());
    assert!(schema["properties"].get("iteration").is_some());
}

#[test]
fn parse_wait_request_clamps_duration_and_iteration() {
    let request = parse_wait_request(&json!({
        "message": "check subagents",
        "duration_secs": 9999,
        "iteration": 0
    }))
    .unwrap();
    assert_eq!(request.duration_ms, MAX_DURATION_SECS * MILLIS_PER_SEC);
    assert_eq!(request.iteration, 1);
}

#[test]
fn missing_message_is_rejected() {
    let err = parse_wait_request(&json!({ "duration_secs": 1 })).unwrap_err();
    assert!(err.contains("message"));
}

#[test]
fn wait_loop_tick_repeats_same_message() {
    let request = parse_wait_request(&json!({
        "message": "poll async workers",
        "duration_ms": 10,
        "loop_key": "workers",
        "iteration": 2
    }))
    .unwrap();
    let output = format_wait_tick(&request, true);

    assert!(output.contains("Loop tick 2 elapsed"));
    assert!(output.contains("\"tool\":\"wait_loop\""));
    assert!(output.contains("\"message\":\"poll async workers\""));
    assert!(output.contains("\"iteration\":3"));
}

#[test]
fn wait_loop_tick_at_max_iteration_does_not_overflow() {
    let request = parse_wait_request(&json!({
        "message": "poll",
        "duration_ms": 10,
        "iteration": u64::MAX
    }))
    .unwrap();
    let output = format_wait_tick(&request, true);
    assert!(output.contains(&format!("\"iteration\":{}", u64::MAX)));
}

#[test]
fn wait_loop_tick_omits_an_absent_loop_key() {
    let request = parse_wait_request(&json!({ "message": "poll", "duration_ms": 10 })).unwrap();
    let output = format_wait_tick(&request, true);
    let json = output
        .split("[wait_tick]\n")
        .nth(1)
        .and_then(|rest| rest.split("\n[/wait_tick]").next())
        .expect("wait_tick payload");
    let payload: serde_json::Value = serde_json::from_str(json).unwrap();
    let arguments = &payload["instructions"]["repeat"]["arguments"];
    assert!(arguments.is_object());
    assert!(
        arguments.get("loop_key").is_none(),
        "the repeat call must not carry a null loop_key: {arguments}"
    );
}

#[tokio::test]
async fn wait_execute_returns_callback_message() {
    let res = WaitTool::new()
        .execute(json!({
            "message": "time to check",
            "duration_ms": 1
        }))
        .await
        .unwrap();
    assert!(!res.is_error);
    assert!(res.output().contains("[wait_tick]"));
    assert!(res.output().contains("time to check"));
}

/// Wire shape is a contract with prompts and recorded transcripts: names and
/// schemas must stay byte-identical across moves.
#[test]
fn wire_schemas_are_stable() {
    assert_eq!(WaitTool::new().name(), "wait");
    assert_eq!(WaitLoopTool::new().name(), "wait_loop");
    let base = json!({
        "type": "object",
        "required": ["message"],
        "properties": {
            "message": {"type": "string", "description": "Callback message to return to the orchestrator after the wait elapses."},
            "duration_secs": {"type": "integer", "minimum": 1, "maximum": 600, "description": "Seconds to wait before returning the callback. Default 5. Ignored when duration_ms is supplied."},
            "duration_ms": {"type": "integer", "minimum": 1, "maximum": 600000, "description": "Milliseconds to wait before returning the callback. Use only for short test-sized waits."}
        }
    });
    assert_eq!(WaitTool::new().parameters_schema(), base);
    let mut looped = base.clone();
    looped["properties"]["loop_key"] = json!({"type": "string", "description": "Optional caller-defined key for the polling loop."});
    looped["properties"]["iteration"] = json!({"type": "integer", "minimum": 1, "description": "Current loop iteration. Defaults to 1; the returned instruction increments it."});
    assert_eq!(WaitLoopTool::new().parameters_schema(), looped);
}
