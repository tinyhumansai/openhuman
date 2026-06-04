//! End-to-end test of the Claude Code stream-json pipeline.
//!
//! Feeds a captured representative CC 2.x stream-json transcript through
//! `StreamJsonParser` → `EventMapper` and asserts that:
//! - text deltas arrive in order and aggregate into the final response
//! - tool-use blocks emit ToolCallStart + ToolCallArgsDelta + a final
//!   ToolCall with parsed JSON arguments
//! - the `result` event finalizes usage tokens (incl. cache_read)
//! - session_id is captured from the first `system` event
//!
//! This is a parser-level E2E; the real driver / process spawn is mocked
//! in `tests/claude_code_driver_smoke.rs`.

use openhuman_core::openhuman::inference::provider::claude_code::{
    event_mapper::EventMapper, stream_parser::StreamJsonParser,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderDelta;

const TRANSCRIPT: &str = r#"{"type":"system","subtype":"init","session_id":"f47ac10b-58cc-4372-a567-0e02b2c3d479","schema_version":"2.0"}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}}
{"type":"stream_event","event":{"type":"content_block_stop","index":0}}
{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_42","name":"memory_search"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"que"}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ry\":\"foo\"}"}}}
{"type":"stream_event","event":{"type":"content_block_stop","index":1}}
{"type":"assistant","message":{"type":"message","role":"assistant","content":[]}}
{"type":"result","subtype":"success","usage":{"input_tokens":120,"output_tokens":42,"cache_read_input_tokens":80,"cache_creation_input_tokens":0},"total_cost_usd":0.0012}
"#;

#[test]
fn captures_text_tool_call_and_usage() {
    let mut parser = StreamJsonParser::new();
    let mut mapper = EventMapper::new();
    let mut deltas: Vec<ProviderDelta> = Vec::new();

    // Feed in chunks to exercise the chunk-boundary buffering as well.
    let mid = TRANSCRIPT.len() / 2;
    for chunk in [&TRANSCRIPT[..mid], &TRANSCRIPT[mid..]] {
        for evt in parser.feed(chunk) {
            for d in mapper.handle(evt) {
                deltas.push(d);
            }
        }
    }
    for evt in parser.end() {
        for d in mapper.handle(evt) {
            deltas.push(d);
        }
    }

    // Schema version was captured by the parser.
    assert_eq!(parser.schema_version.as_deref(), Some("2.0"));

    // Session id was captured by the mapper from the first system event.
    assert_eq!(
        mapper.session_id.as_deref(),
        Some("f47ac10b-58cc-4372-a567-0e02b2c3d479")
    );

    // Text deltas arrived in order.
    let text_chunks: Vec<&str> = deltas
        .iter()
        .filter_map(|d| match d {
            ProviderDelta::TextDelta { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_chunks, vec!["Hello", " world"]);

    // Tool call lifecycle.
    assert!(deltas.iter().any(|d| matches!(
        d,
        ProviderDelta::ToolCallStart { tool_name, call_id }
            if tool_name == "memory_search" && call_id == "call_42"
    )));
    let args_concat: String = deltas
        .iter()
        .filter_map(|d| match d {
            ProviderDelta::ToolCallArgsDelta { call_id, delta } if call_id == "call_42" => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(args_concat, r#"{"query":"foo"}"#);

    // Aggregated response.
    assert_eq!(mapper.final_text, "Hello world");
    assert_eq!(mapper.tool_calls.len(), 1);
    assert_eq!(mapper.tool_calls[0].name, "memory_search");
    assert_eq!(mapper.tool_calls[0].id, "call_42");
    assert_eq!(mapper.tool_calls[0].arguments, r#"{"query":"foo"}"#);

    // Usage from the `result` event.
    assert!(mapper.finished);
    let u = mapper.usage.as_ref().expect("usage should be populated");
    assert_eq!(u.input_tokens, 120);
    assert_eq!(u.output_tokens, 42);
    assert_eq!(u.cached_input_tokens, 80);
}
