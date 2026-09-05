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

#[test]
fn tool_call_assembles_input() {
    let mut m = EventMapper::new();
    let start = json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_1","name":"memory_search"}});
    let d_args = json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"foo\"}"}});
    let stop = json!({"type":"content_block_stop","index":1});
    let starts = m.handle(ClaudeCodeEvent::StreamEvent { event: start });
    assert!(
        matches!(&starts[0], ProviderDelta::ToolCallStart { tool_name, .. } if tool_name == "memory_search")
    );
    let args = m.handle(ClaudeCodeEvent::StreamEvent { event: d_args });
    assert!(matches!(&args[0], ProviderDelta::ToolCallArgsDelta { .. }));
    m.handle(ClaudeCodeEvent::StreamEvent { event: stop });
    assert_eq!(m.tool_calls.len(), 1);
    assert_eq!(m.tool_calls[0].name, "memory_search");
    assert_eq!(m.tool_calls[0].arguments, r#"{"q":"foo"}"#);
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
