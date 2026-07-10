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
        reasoning_content: None,
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
        tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
            id: "tc1".into(),
            name: "file_read".into(),
            arguments: "{\"path\":\"a.txt\"}".into(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: None,
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
        reasoning_content: None,
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
        reasoning_content: None,
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
        reasoning_content: None,
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
        reasoning_content: None,
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
        reasoning_content: None,
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

// ── TAURI-RUST-7 regression: tool_calls / ToolResults pairing ──────────
//
// Providers reject any assistant `tool_calls` message that isn't immediately
// followed by `tool` messages responding to every `tool_call_id`. Cached
// transcript restores and mid-turn aborts can produce bisected pairs. The
// fix in `to_provider_messages` drops unpaired AssistantToolCalls and orphan
// ToolResults so the wire payload is always well-formed.

fn assistant_tool_calls(id: &str) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: Some("calling tool".into()),
        tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: "{}".into(),
            extra_content: None,
        }],
        reasoning_content: None,
        extra_metadata: None,
    }
}

fn tool_results(id: &str) -> ConversationMessage {
    use crate::openhuman::inference::provider::ToolResultMessage;
    ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: id.into(),
        content: "ok".into(),
    }])
}

fn user_chat(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(crate::openhuman::inference::provider::ChatMessage::user(
        text,
    ))
}

fn assistant_chat(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(crate::openhuman::inference::provider::ChatMessage::assistant(text))
}

#[test]
fn to_provider_messages_keeps_paired_tool_cycle() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls("tc-1"),
        tool_results("tc-1"),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
}

#[test]
fn to_provider_messages_drops_trailing_unpaired_tool_calls() {
    // The assistant emitted tool_calls but the run was aborted before the
    // ToolResults were persisted. The trailing tool_calls must not reach
    // the wire.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_chat("ok"),
        user_chat("again"),
        assistant_tool_calls("tc-2"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "user"],
        "trailing unpaired AssistantToolCalls must be stripped"
    );
}

#[test]
fn to_provider_messages_drops_mid_history_unpaired_tool_calls() {
    // History with a bisected pair in the middle: tool_calls followed
    // directly by a Chat (not ToolResults). Drop the tool_calls; keep
    // everything else.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls("tc-3"), // bisected — no following ToolResults
        user_chat("nevermind"),
        assistant_chat("ok"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "user", "assistant"]);
}

#[test]
fn to_provider_messages_drops_orphan_tool_results() {
    // Symmetric drop: ToolResults whose preceding AssistantToolCalls was
    // never emitted (either never persisted or already dropped above)
    // must not appear in the wire payload.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        tool_results("tc-4"), // orphan — no preceding AssistantToolCalls
        assistant_chat("ok"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant"]);
}

#[test]
fn to_provider_messages_handles_multiple_tool_cycles() {
    // Two paired cycles in a row — both must survive.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("a"),
        assistant_tool_calls("tc-5"),
        tool_results("tc-5"),
        assistant_tool_calls("tc-6"),
        tool_results("tc-6"),
        assistant_chat("final"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec![
            "user",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant"
        ]
    );
}

// ── tool_call_id set-pairing (CodeRabbit follow-up) ─────────────────────

fn assistant_tool_calls_multi(ids: &[&str]) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: Some("calling tools".into()),
        tool_calls: ids
            .iter()
            .map(|id| crate::openhuman::inference::provider::ToolCall {
                id: (*id).into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            })
            .collect(),
        reasoning_content: None,
        extra_metadata: None,
    }
}

#[test]
fn native_dispatcher_serializes_reasoning_content_for_tool_call_turns() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        ConversationMessage::AssistantToolCalls {
            text: Some("calling tools".into()),
            tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                id: "tc-1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            }],
            reasoning_content: Some("chain-of-thought replay blob".into()),
            extra_metadata: None,
        },
        tool_results("tc-1"),
    ];

    let out = dispatcher.to_provider_messages(&history);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].role, "assistant");

    let payload: serde_json::Value =
        serde_json::from_str(&out[0].content).expect("assistant payload should be valid JSON");
    assert_eq!(
        payload
            .get("reasoning_content")
            .and_then(serde_json::Value::as_str),
        Some("chain-of-thought replay blob")
    );
}

#[test]
fn native_dispatcher_omits_reasoning_content_when_absent() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![assistant_tool_calls("tc-1"), tool_results("tc-1")];

    let out = dispatcher.to_provider_messages(&history);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].role, "assistant");

    let payload: serde_json::Value =
        serde_json::from_str(&out[0].content).expect("assistant payload should be valid JSON");
    assert!(
        payload.get("reasoning_content").is_none(),
        "reasoning_content should be omitted when absent"
    );
}

fn tool_results_multi(ids: &[&str]) -> ConversationMessage {
    use crate::openhuman::inference::provider::ToolResultMessage;
    ConversationMessage::ToolResults(
        ids.iter()
            .map(|id| ToolResultMessage {
                tool_call_id: (*id).into(),
                content: "ok".into(),
            })
            .collect(),
    )
}

#[test]
fn to_provider_messages_drops_pair_when_tool_call_ids_mismatch() {
    // Opener requests `tc-1`, but the only ToolResults entry answers `tc-x`.
    // Backend would 400 with "insufficient tool messages following tool_calls"
    // — drop both.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1"]),
        tool_results_multi(&["tc-x"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "id-set mismatch must drop the bisected pair entirely, kept: {roles:?}"
    );
}

#[test]
fn to_provider_messages_drops_pair_when_results_are_partial() {
    // Opener requests two tool_call_ids, results answer only one. Backend
    // rejects with "insufficient tool messages". Strict set equality drops
    // the pair.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1", "tc-2"]),
        tool_results_multi(&["tc-1"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "partial tool-result coverage must drop the pair, kept: {roles:?}"
    );
}

#[test]
fn to_provider_messages_keeps_pair_with_full_id_coverage() {
    // Strict set equality: opener has {tc-1, tc-2}, results cover both,
    // even if listed in a different order. Both messages must be emitted.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1", "tc-2"]),
        tool_results_multi(&["tc-2", "tc-1"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "tool", "tool", "assistant"]
    );
}

#[test]
fn to_provider_messages_drops_pair_with_extra_unsolicited_results() {
    // Opener requests `tc-1`; results answer `tc-1` *and* an unsolicited
    // `tc-extra`. The id sets differ, so the pair is dropped.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1"]),
        tool_results_multi(&["tc-1", "tc-extra"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "extra unsolicited tool_call_ids must invalidate the pair, kept: {roles:?}"
    );
}
