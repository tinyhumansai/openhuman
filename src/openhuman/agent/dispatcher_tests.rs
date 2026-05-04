use super::*;
use crate::openhuman::agent::pformat::PFormatToolParams;

#[test]
fn xml_dispatcher_parses_tool_calls() {
    let response = ChatResponse {
        text: Some(
            "Checking\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let dispatcher = XmlToolDispatcher;
    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn native_dispatcher_roundtrip() {
    let response = ChatResponse {
        text: Some("ok".into()),
        tool_calls: vec![crate::openhuman::providers::ToolCall {
            id: "tc1".into(),
            name: "file_read".into(),
            arguments: "{\"path\":\"a.txt\"}".into(),
        }],
        usage: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_call_id.as_deref(), Some("tc1"));

    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "file_read".into(),
        output: "hello".into(),
        success: true,
        tool_call_id: Some("tc1".into()),
    }]);
    match msg {
        ConversationMessage::ToolResults(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_call_id, "tc1");
        }
        _ => panic!("expected tool results"),
    }
}

#[test]
fn native_dispatcher_falls_back_to_xml_tool_calls() {
    let response = ChatResponse {
        text: Some(
            "Checking files...\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Checking files...");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].tool_call_id, None);
}

#[test]
fn native_dispatcher_falls_back_to_invoke_tag() {
    let response = ChatResponse {
        text: Some(
            "Let me run this.\n<invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}</invoke>".into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Let me run this.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn xml_format_results_contains_tool_result_tags() {
    let dispatcher = XmlToolDispatcher;
    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "shell".into(),
        output: "ok".into(),
        success: true,
        tool_call_id: None,
    }]);
    let rendered = match msg {
        ConversationMessage::Chat(chat) => chat.content,
        _ => String::new(),
    };
    assert!(rendered.contains("<tool_result"));
    assert!(rendered.contains("shell"));
}

fn pformat_registry_for(name: &str, props: serde_json::Value) -> PFormatRegistry {
    let schema = serde_json::json!({
        "type": "object",
        "properties": props
    });
    let mut reg = PFormatRegistry::new();
    reg.insert(name.to_string(), PFormatToolParams::from_schema(&schema));
    reg
}

#[test]
fn pformat_dispatcher_parses_tool_call_tag() {
    // The model emits a p-format call inside a `<tool_call>` tag.
    // The dispatcher should pull it out, look up the tool's
    // parameter ordering, and produce named JSON args.
    let registry = pformat_registry_for(
        "get_weather",
        serde_json::json!({
            "location": { "type": "string" },
            "unit": { "type": "string" }
        }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Let me check the weather.\n<tool_call>get_weather[London|metric]</tool_call>".into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Let me check the weather.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_weather");
    assert_eq!(
        calls[0].arguments,
        serde_json::json!({"location": "London", "unit": "metric"})
    );
}

#[test]
fn pformat_dispatcher_falls_back_to_json_in_tag() {
    // A model that ignored the p-format protocol and emitted a
    // JSON tool call should still be parsed correctly — the
    // dispatcher's whole point is to be a strict superset of the
    // legacy XML behaviour.
    let registry = pformat_registry_for(
        "shell",
        serde_json::json!({ "command": { "type": "string" } }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Running it now.\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Running it now.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
}

#[test]
fn pformat_dispatcher_handles_multiple_tags() {
    let registry = pformat_registry_for(
        "shell",
        serde_json::json!({ "command": { "type": "string" } }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Step 1.\n<tool_call>shell[ls]</tool_call>\nStep 2.\n<tool_call>shell[pwd]</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    };
    let (_text, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    assert_eq!(calls[1].arguments, serde_json::json!({"command": "pwd"}));
}

#[test]
fn pformat_dispatcher_reports_pformat_tool_call_format() {
    let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
    assert_eq!(dispatcher.tool_call_format(), ToolCallFormat::PFormat);
}

#[test]
fn pformat_dispatcher_instructions_are_protocol_only() {
    // The dispatcher's prompt_instructions should NOT re-render
    // the tool catalogue — that's `ToolsSection`'s job. Otherwise
    // every tool gets emitted twice and the prompt double-pays.
    let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
    // Pass in a tool to make sure the dispatcher ignores it.
    struct DummyTool;
    #[async_trait::async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "should_not_appear"
        }
        fn description(&self) -> &str {
            "this string must not show up in the dispatcher instructions"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
            Ok(crate::openhuman::tools::ToolResult::success("ok"))
        }
    }
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(DummyTool)];
    let instructions = dispatcher.prompt_instructions(&tools);
    assert!(instructions.contains("Tool Use Protocol"));
    assert!(
        !instructions.contains("should_not_appear"),
        "dispatcher instructions must not duplicate the tool catalogue, got:\n{instructions}"
    );
}

#[test]
fn native_format_results_keeps_tool_call_id() {
    let dispatcher = NativeToolDispatcher;
    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "shell".into(),
        output: "ok".into(),
        success: true,
        tool_call_id: Some("tc-1".into()),
    }]);

    match msg {
        ConversationMessage::ToolResults(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_call_id, "tc-1");
        }
        _ => panic!("expected ToolResults variant"),
    }
}
