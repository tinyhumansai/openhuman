use super::*;
use serde_json::json;

fn text_block_start(idx: u64) -> Value {
    json!({"type":"content_block_start","index":idx,"content_block":{"type":"text"}})
}
fn text_delta(idx: u64, t: &str) -> Value {
    json!({"type":"content_block_delta","index":idx,"delta":{"type":"text_delta","text":t}})
}

#[test]
fn text_streams_through() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::StreamEvent {
        event: text_block_start(0),
    });
    let d1 = m.handle(ClaudeCodeEvent::StreamEvent {
        event: text_delta(0, "hel"),
    });
    let d2 = m.handle(ClaudeCodeEvent::StreamEvent {
        event: text_delta(0, "lo"),
    });
    assert!(matches!(&d1[0], ProviderDelta::TextDelta { delta } if delta == "hel"));
    assert!(matches!(&d2[0], ProviderDelta::TextDelta { delta } if delta == "lo"));
    assert_eq!(m.final_text, "hello");
}

/// A native `tool_use` block is the CLI's own call — a builtin, or a server
/// from the `--mcp-config` it was handed — and the CLI runs it inside its own
/// loop. The matching `tool_result` was always dropped; surfacing the call
/// while dropping its result handed OpenHuman's harness a tool it cannot run,
/// which failed repeatedly until the circuit breaker halted the whole turn.
/// Neither half is surfaced now.
#[test]
fn cli_internal_tool_calls_are_not_surfaced_to_the_harness() {
    let mut m = EventMapper::new();
    let start = json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_1","name":"Bash"}});
    let d_args = json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}});
    let stop = json!({"type":"content_block_stop","index":1});

    assert!(
        m.handle(ClaudeCodeEvent::StreamEvent { event: start })
            .is_empty(),
        "no ToolCallStart reaches the harness"
    );
    assert!(
        m.handle(ClaudeCodeEvent::StreamEvent { event: d_args })
            .is_empty(),
        "no ToolCallArgsDelta reaches the harness"
    );
    m.handle(ClaudeCodeEvent::StreamEvent { event: stop });

    assert!(
        m.tool_calls.is_empty(),
        "the aggregated response carries no tool calls for the harness to execute"
    );
}

#[test]
fn cli_internal_tool_blocks_without_metadata_are_still_suppressed() {
    let mut m = EventMapper::new();
    let start = json!({
        "type": "content_block_start",
        "index": 1,
        "content_block": {"type": "tool_use"}
    });
    let args = json!({
        "type": "content_block_delta",
        "index": 1,
        "delta": {"type": "input_json_delta", "partial_json": "{\"x\":1}"}
    });
    let stop = json!({"type": "content_block_stop", "index": 1});

    assert!(m
        .handle(ClaudeCodeEvent::StreamEvent { event: start })
        .is_empty());
    assert!(m
        .handle(ClaudeCodeEvent::StreamEvent { event: args })
        .is_empty());
    assert!(m
        .handle(ClaudeCodeEvent::StreamEvent { event: stop })
        .is_empty());
    assert!(m.tool_calls.is_empty());
}

/// Text in the same turn still streams normally — dropping the tool block must
/// not swallow the CLI's actual answer.
#[test]
fn text_in_a_turn_with_a_cli_tool_call_still_streams() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::StreamEvent {
        event: json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"Write"}}),
    });
    m.handle(ClaudeCodeEvent::StreamEvent {
        event: json!({"type":"content_block_stop","index":0}),
    });
    m.handle(ClaudeCodeEvent::StreamEvent {
        event: text_block_start(1),
    });
    let deltas = m.handle(ClaudeCodeEvent::StreamEvent {
        event: text_delta(1, "done"),
    });

    assert!(matches!(&deltas[0], ProviderDelta::TextDelta { delta } if delta == "done"));
    assert_eq!(m.final_text, "done");
    assert!(m.tool_calls.is_empty());
}

#[test]
fn result_event_captures_usage() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("success".into()),
        is_error: false,
        usage: Some(json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 25
        })),
        total_cost_usd: Some(0.001),
        raw: Value::Null,
    });
    assert!(m.finished);
    let u = m.usage.as_ref().unwrap();
    assert_eq!(u.input_tokens, 100);
    assert_eq!(u.output_tokens, 50);
    assert_eq!(u.cached_input_tokens, 25);
    // cost wired through from total_cost_usd
    assert!((u.charged_amount_usd - 0.001).abs() < f64::EPSILON);
}

#[test]
fn cost_surfaced_even_without_usage_object() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("success".into()),
        is_error: false,
        usage: None,
        total_cost_usd: Some(0.05),
        raw: Value::Null,
    });
    let u = m
        .usage
        .as_ref()
        .expect("usage synthesized for cost-only result");
    assert_eq!(u.input_tokens, 0);
    assert!((u.charged_amount_usd - 0.05).abs() < f64::EPSILON);
}

#[test]
fn final_assistant_message_is_skipped() {
    let mut m = EventMapper::new();
    let deltas = m.handle(ClaudeCodeEvent::Assistant {
        message: json!({"type":"message","role":"assistant","content":[]}),
    });
    assert!(deltas.is_empty());
}

// Carried over from #5713 (@Felyx-Fu), which was closed in favour of this PR.

#[test]
fn empty_cli_error_does_not_override_stderr_fallback() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Error {
        message: String::new(),
    });

    assert!(m.error.is_none());
    assert!(m.terminal_error);
}

#[test]
fn structured_cli_error_is_preserved() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Error {
        message: "structured failure".into(),
    });

    assert_eq!(m.error.as_deref(), Some("structured failure"));
    assert!(m.terminal_error);
}

#[test]
fn cli_error_redacts_sensitive_diagnostics() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Error {
        message: "request failed with sk-secret-token".into(),
    });

    assert_eq!(m.error.as_deref(), Some("request failed with [REDACTED]"));
}

#[test]
fn result_error_is_recorded_without_masking_stderr_fallback() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("error".into()),
        is_error: false,
        usage: None,
        total_cost_usd: None,
        raw: Value::Null,
    });

    assert!(m.finished);
    assert!(m.terminal_error);
    assert!(m.error.is_none());
}

#[test]
fn result_is_error_flag_marks_terminal_failure() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("success".into()),
        is_error: true,
        usage: None,
        total_cost_usd: None,
        raw: Value::Null,
    });

    assert!(m.finished);
    assert!(m.terminal_error);
    assert!(m.error.is_none());
}

#[test]
fn failed_result_preserves_errors_payload() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("error_during_execution".into()),
        is_error: true,
        usage: None,
        total_cost_usd: None,
        raw: json!({
            "errors": ["API request failed", "retry exhausted"],
            "result": "The provider could not complete the request"
        }),
    });

    assert_eq!(
        m.error.as_deref(),
        Some("API request failed; retry exhausted; The provider could not complete the request")
    );
}

#[test]
fn failed_result_preserves_nested_error_message() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("error".into()),
        is_error: false,
        usage: None,
        total_cost_usd: None,
        raw: json!({"error": {"message": "authentication failed"}}),
    });

    assert_eq!(m.error.as_deref(), Some("authentication failed"));
}

#[test]
fn result_diagnostic_redacts_sensitive_values() {
    let mut m = EventMapper::new();
    m.handle(ClaudeCodeEvent::Result {
        subtype: Some("error".into()),
        is_error: true,
        usage: None,
        total_cost_usd: None,
        raw: json!({"error": {"message": "token sk-secret-token was rejected"}}),
    });

    assert_eq!(m.error.as_deref(), Some("token [REDACTED] was rejected"));
}
