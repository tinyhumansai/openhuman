use super::*;

#[test]
fn parses_multiline_chunk() {
    let mut p = StreamJsonParser::new();
    let chunk = r#"{"type":"system","session_id":"s1","schema_version":"2.0"}
{"type":"assistant","message":{"type":"content_block_start","index":0,"content_block":{"type":"text"}}}
"#;
    let events = p.feed(chunk);
    assert_eq!(events.len(), 2);
    assert_eq!(p.schema_version.as_deref(), Some("2.0"));
    assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
    assert!(matches!(events[1], ClaudeCodeEvent::Assistant { .. }));
}

#[test]
fn handles_split_lines_across_chunks() {
    let mut p = StreamJsonParser::new();
    assert!(p.feed("{\"type\":\"system\"").is_empty());
    assert!(p.feed(",\"session_id\":\"s1\"}").is_empty());
    let events = p.feed("\n");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
}

#[test]
fn flushes_trailing_line_on_end() {
    let mut p = StreamJsonParser::new();
    assert!(p
        .feed(r#"{"type":"result","subtype":"success"}"#)
        .is_empty());
    let events = p.end();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], ClaudeCodeEvent::Result { .. }));
}

#[test]
fn unknown_type_becomes_parse_error() {
    let mut p = StreamJsonParser::new();
    let events = p.feed("{\"type\":\"weird\"}\n");
    assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
}

#[test]
fn bad_json_becomes_parse_error() {
    let mut p = StreamJsonParser::new();
    let events = p.feed("not json\n");
    assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
}

// The three cases below carry over @Felyx-Fu's work from #5713, which was closed
// in favour of this PR.

#[test]
fn parses_terminal_is_error_flag() {
    let mut p = StreamJsonParser::new();
    let events = p.feed(
        r#"{"type":"result","subtype":"success","is_error":true}
"#,
    );

    assert!(matches!(
        &events[0],
        ClaudeCodeEvent::Result { is_error: true, .. }
    ));
}

#[test]
fn parses_nested_error_message() {
    let mut p = StreamJsonParser::new();
    let events = p.feed(
        r#"{"type":"error","error":{"message":"structured failure","type":"invalid_request"}}
"#,
    );

    assert!(matches!(
        &events[0],
        ClaudeCodeEvent::Error { message } if message == "structured failure"
    ));
}

#[test]
fn missing_error_message_does_not_create_generic_diagnostic() {
    let mut p = StreamJsonParser::new();
    let events = p.feed(
        r#"{"type":"error","error":{"type":"unknown"}}
"#,
    );

    assert!(matches!(
        &events[0],
        ClaudeCodeEvent::Error { message } if message.is_empty()
    ));
}

#[test]
fn empty_nested_error_message_falls_back_to_top_level_message() {
    let mut p = StreamJsonParser::new();
    let events = p.feed(
        r#"{"type":"error","error":{"message":"  "},"message":"top-level failure"}
"#,
    );

    assert!(matches!(
        &events[0],
        ClaudeCodeEvent::Error { message } if message == "top-level failure"
    ));
}
