use super::*;
use crate::openhuman::tools::ToolResult;
use async_trait::async_trait;

struct StubTool(&'static str);

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "stub tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

#[test]
fn parse_argument_helpers_cover_string_non_string_and_missing_values() {
    assert_eq!(
        parse_arguments_value(Some(&serde_json::json!("{\"value\":1}"))),
        serde_json::json!({ "value": 1 })
    );
    assert_eq!(
        parse_arguments_value(Some(&serde_json::json!("not-json"))),
        serde_json::json!({})
    );
    assert_eq!(
        parse_arguments_value(Some(&serde_json::json!({ "value": 2 }))),
        serde_json::json!({ "value": 2 })
    );
    assert_eq!(parse_arguments_value(None), serde_json::json!({}));
}

#[test]
fn parse_tool_call_value_supports_function_shape_flat_shape_and_invalid_names() {
    let function_shape = serde_json::json!({
        "function": {
            "name": "shell",
            "arguments": "{\"command\":\"ls\"}"
        }
    });
    let parsed = parse_tool_call_value(&function_shape).expect("function call should parse");
    assert_eq!(parsed.name, "shell");
    assert_eq!(parsed.arguments, serde_json::json!({ "command": "ls" }));

    let flat_shape = serde_json::json!({
        "name": "echo",
        "arguments": { "value": "hi" }
    });
    let parsed = parse_tool_call_value(&flat_shape).expect("flat call should parse");
    assert_eq!(parsed.name, "echo");
    assert_eq!(parsed.arguments, serde_json::json!({ "value": "hi" }));

    assert!(parse_tool_call_value(&serde_json::json!({ "name": "   " })).is_none());
    assert!(parse_tool_call_value(&serde_json::json!({ "function": {} })).is_none());
}

#[test]
fn parse_tool_calls_from_json_value_handles_tool_calls_array_arrays_and_singletons() {
    let wrapped = serde_json::json!({
        "tool_calls": [
            { "name": "echo", "arguments": { "value": "one" } },
            { "function": { "name": "shell", "arguments": "{\"command\":\"pwd\"}" } }
        ],
        "content": "assistant text"
    });
    let calls = parse_tool_calls_from_json_value(&wrapped);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "echo");
    assert_eq!(calls[1].name, "shell");

    let array = serde_json::json!([
        { "name": "echo", "arguments": { "value": "two" } },
        { "name": "   " }
    ]);
    let calls = parse_tool_calls_from_json_value(&array);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, serde_json::json!({ "value": "two" }));

    let single = serde_json::json!({ "name": "echo", "arguments": { "value": "three" } });
    let calls = parse_tool_calls_from_json_value(&single);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn tag_and_json_extractors_cover_common_edge_cases() {
    assert_eq!(
        find_first_tag("hi <invoke>there", &["<tool_call>", "<invoke>"]),
        Some((3, "<invoke>"))
    );
    assert_eq!(
        matching_tool_call_close_tag("<toolcall>"),
        Some("</toolcall>")
    );
    assert_eq!(matching_tool_call_close_tag("<nope>"), None);

    let extracted = extract_first_json_value_with_end(" text {\"ok\":true} trailing ")
        .expect("json should be found");
    assert_eq!(extracted.0, serde_json::json!({ "ok": true }));
    assert!(extracted.1 > 0);

    assert_eq!(
        strip_leading_close_tags(" </tool_call>  </invoke> hi "),
        "hi "
    );
    assert_eq!(strip_leading_close_tags("plain"), "plain");

    let values = extract_json_values("before {\"a\":1} [1,2] after");
    assert_eq!(
        values,
        vec![serde_json::json!({ "a": 1 }), serde_json::json!([1, 2])]
    );

    assert_eq!(
        find_json_end("  {\"a\":\"}\"}tail"),
        Some("  {\"a\":\"}\"}".len())
    );
    assert_eq!(find_json_end("[1,2,3]"), None);
}

#[test]
fn glm_helpers_parse_aliases_urls_and_commands() {
    assert_eq!(map_glm_tool_alias("browser_open"), "shell");
    assert_eq!(map_glm_tool_alias("http"), "http_request");
    assert_eq!(map_glm_tool_alias("custom_tool"), "custom_tool");

    assert_eq!(
        build_curl_command("https://example.com?q=1"),
        Some("curl -s 'https://example.com?q=1'".into())
    );
    assert_eq!(
        build_curl_command("https://exa'mple.com"),
        Some("curl -s 'https://exa'\\\\''mple.com'".into())
    );
    assert!(build_curl_command("ftp://example.com").is_none());
    assert!(build_curl_command("https://example.com/has space").is_none());

    let calls = parse_glm_style_tool_calls(
        "browser_open/url>https://example.com\nhttp_request/url>https://api.example.com\nplain text\nhttps://rust-lang.org",
    );
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].0, "shell");
    assert_eq!(calls[1].0, "http_request");
    assert_eq!(calls[2].0, "shell");
}

#[test]
fn parse_tool_calls_supports_native_json_xml_markdown_and_glm_formats() {
    let native = serde_json::json!({
        "content": "native text",
        "tool_calls": [
            { "name": "echo", "arguments": { "value": "one" } }
        ]
    })
    .to_string();
    let (text, calls) = parse_tool_calls(&native);
    assert_eq!(text, "native text");
    assert_eq!(calls.len(), 1);

    let xml = "before\n<tool_call>\n{\"name\":\"echo\",\"arguments\":{\"value\":\"two\"}}\n</tool_call>\nafter";
    let (text, calls) = parse_tool_calls(xml);
    assert_eq!(text, "before\nafter");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, serde_json::json!({ "value": "two" }));

    let unclosed = "<invoke>{\"name\":\"echo\",\"arguments\":{\"value\":\"three\"}}</invoke>";
    let (text, calls) = parse_tool_calls(unclosed);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);

    let markdown =
        "lead\n```tool_call\n{\"name\":\"echo\",\"arguments\":{\"value\":\"four\"}}\n```\ntrail";
    let (text, calls) = parse_tool_calls(markdown);
    assert_eq!(text, "lead\ntrail");
    assert_eq!(calls.len(), 1);

    let glm = "shell/command>ls -la";
    let (text, calls) = parse_tool_calls(glm);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn structured_tool_call_and_history_helpers_round_trip_expected_shapes() {
    let tool_calls = vec![ToolCall {
        id: "call-1".into(),
        name: "echo".into(),
        arguments: "{\"value\":\"hello\"}".into(),
    }];

    let parsed = parse_structured_tool_calls(&tool_calls);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].arguments, serde_json::json!({ "value": "hello" }));

    let native = build_native_assistant_history("done", &tool_calls);
    let native_json: serde_json::Value = serde_json::from_str(&native).expect("valid json");
    assert_eq!(native_json["content"], "done");
    assert_eq!(native_json["tool_calls"][0]["id"], "call-1");

    let xml_history = build_assistant_history_with_tool_calls("", &tool_calls);
    assert!(xml_history.contains("<tool_call>"));
    assert!(xml_history.contains("\"name\":\"echo\""));
}

#[test]
fn tools_to_openai_format_uses_tool_metadata() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(StubTool("echo")), Box::new(StubTool("shell"))];
    let payload = tools_to_openai_format(&tools);

    assert_eq!(payload.len(), 2);
    assert_eq!(payload[0]["type"], "function");
    assert_eq!(payload[0]["function"]["name"], "echo");
    assert_eq!(payload[1]["function"]["description"], "stub tool");
}
