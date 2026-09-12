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
