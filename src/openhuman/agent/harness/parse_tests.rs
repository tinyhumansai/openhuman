//! Tests for the OpenHuman-specific half of tool-call parsing.
//!
//! The parser tests moved with the parsers, to
//! `tinyagents_harness::tool_calling`. What is exercised here is the wire
//! vocabulary that stayed: OpenHuman's `ToolCall`, its native-history JSON
//! (including the Gemini `thought_signature` round-trip), and its OpenAI
//! function-calling payload.

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
fn structured_tool_call_and_history_helpers_round_trip_expected_shapes() {
    let tool_calls = vec![ToolCall {
        id: "call-1".into(),
        name: "echo".into(),
        arguments: "{\"value\":\"hello\"}".into(),
        extra_content: None,
    }];

    let parsed = parse_structured_tool_calls(&tool_calls);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].arguments, serde_json::json!({ "value": "hello" }));

    let native = build_native_assistant_history("done", None, &tool_calls);
    let native_json: serde_json::Value = serde_json::from_str(&native).expect("valid json");
    assert_eq!(native_json["content"], "done");
    assert_eq!(native_json["tool_calls"][0]["id"], "call-1");
    // No reasoning supplied -> field omitted entirely (non-reasoning models
    // must not gain a spurious `reasoning_content` key).
    assert!(native_json.get("reasoning_content").is_none());

    // DeepSeek thinking mode: reasoning must round-trip onto the tool-call
    // turn (Sentry TAURI-RUST-4KB).
    let native_reasoning =
        build_native_assistant_history("done", Some("  step-by-step thoughts  "), &tool_calls);
    let reasoning_json: serde_json::Value =
        serde_json::from_str(&native_reasoning).expect("valid json");
    assert_eq!(reasoning_json["reasoning_content"], "step-by-step thoughts");
    // Whitespace-only reasoning is treated as absent.
    let native_blank = build_native_assistant_history("done", Some("   "), &tool_calls);
    let blank_json: serde_json::Value = serde_json::from_str(&native_blank).expect("valid json");
    assert!(blank_json.get("reasoning_content").is_none());

    let xml_history = build_assistant_history_with_tool_calls("", &tool_calls);
    assert!(xml_history.contains("<tool_call>"));
    assert!(xml_history.contains("\"name\":\"echo\""));
}
#[test]
fn build_native_assistant_history_persists_per_call_extra_content() {
    let tool_calls = vec![
        ToolCall {
            id: "call-a".into(),
            name: "shell".into(),
            arguments: "{}".into(),
            extra_content: Some(serde_json::json!({"google":{"thought_signature":"SIG_A"}})),
        },
        ToolCall {
            id: "call-b".into(),
            name: "read".into(),
            arguments: "{}".into(),
            extra_content: Some(serde_json::json!({"google":{"thought_signature":"SIG_B"}})),
        },
        // A call that never had a signature must NOT gain an empty key.
        ToolCall {
            id: "call-c".into(),
            name: "noop".into(),
            arguments: "{}".into(),
            extra_content: None,
        },
    ];

    let native = build_native_assistant_history("on it", None, &tool_calls);
    let json: serde_json::Value = serde_json::from_str(&native).expect("valid json");

    assert_eq!(
        json.pointer("/tool_calls/0/extra_content/google/thought_signature")
            .and_then(|v| v.as_str()),
        Some("SIG_A"),
        "first parallel call's signature must be persisted"
    );
    assert_eq!(
        json.pointer("/tool_calls/1/extra_content/google/thought_signature")
            .and_then(|v| v.as_str()),
        Some("SIG_B"),
        "second parallel call's signature must be persisted (not just the first)"
    );
    assert!(
        json.pointer("/tool_calls/2/extra_content").is_none(),
        "a call without extra_content must omit the field, keeping non-Gemini history byte-identical"
    );
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
