use super::*;

#[test]
fn parse_tool_calls_nested_xml_tags_handled() {
    // Double-wrapped tool call should still parse the inner call
    let response =
        r#"<tool_call><tool_call>{"name":"echo","arguments":{"msg":"hi"}}</tool_call></tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    // Should find at least one tool call
    assert!(
        !calls.is_empty(),
        "nested XML tags should still yield at least one tool call"
    );
}

#[test]
fn parse_tool_calls_truncated_json_no_panic() {
    // Incomplete JSON inside tool_call tags
    let response = r#"<tool_call>{"name":"shell","arguments":{"command":"ls"</tool_call>"#;
    let (_text, _calls) = parse_tool_calls(response);
    // Should not panic — graceful handling of truncated JSON
}

#[test]
fn parse_tool_calls_empty_json_object_in_tag() {
    let response = "<tool_call>{}</tool_call>";
    let (_text, calls) = parse_tool_calls(response);
    // Empty JSON object has no name field — should not produce valid tool call
    assert!(
        calls.is_empty(),
        "empty JSON object should not produce a tool call"
    );
}

#[test]
fn parse_tool_calls_closing_tag_only_returns_text() {
    let response = "Some text </tool_call> more text";
    let (text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "closing tag only should not produce calls"
    );
    assert!(
        !text.is_empty(),
        "text around orphaned closing tag should be preserved"
    );
}

#[test]
fn parse_tool_calls_very_large_arguments_no_panic() {
    let large_arg = "x".repeat(100_000);
    let response = format!(
        r#"<tool_call>{{"name":"echo","arguments":{{"message":"{}"}}}}</tool_call>"#,
        large_arg
    );
    let (_text, calls) = parse_tool_calls(&response);
    assert_eq!(calls.len(), 1, "large arguments should still parse");
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_special_characters_in_arguments() {
    let response = r#"<tool_call>{"name":"echo","arguments":{"message":"hello \"world\" <>&'\n\t"}}</tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_text_with_embedded_json_not_extracted() {
    // Raw JSON without any tags should NOT be extracted as a tool call
    let response = r#"Here is some data: {"name":"echo","arguments":{"message":"hi"}} end."#;
    let (_text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "raw JSON in text without tags should not be extracted"
    );
}

#[test]
fn parse_tool_calls_multiple_formats_mixed() {
    // Mix of text and properly tagged tool call
    let response = r#"I'll help you with that.

<tool_call>
{"name":"shell","arguments":{"command":"echo hello"}}
</tool_call>

Let me check the result."#;
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(
        calls.len(),
        1,
        "should extract one tool call from mixed content"
    );
    assert_eq!(calls[0].name, "shell");
    assert!(
        text.contains("help you"),
        "text before tool call should be preserved"
    );
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): scrub_credentials edge cases
// ─────────────────────────────────────────────────────────────────────

#[test]
fn scrub_credentials_empty_input() {
    let result = scrub_credentials("");
    assert_eq!(result, "");
}

#[test]
fn scrub_credentials_no_sensitive_data() {
    let input = "normal text without any secrets";
    let result = scrub_credentials(input);
    assert_eq!(
        result, input,
        "non-sensitive text should pass through unchanged"
    );
}

#[test]
fn scrub_credentials_short_values_not_redacted() {
    // Values shorter than 8 chars should not be redacted
    let input = r#"api_key="short""#;
    let result = scrub_credentials(input);
    assert_eq!(result, input, "short values should not be redacted");
}
