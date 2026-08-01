use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// 1. Simple text response (no tools)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_returns_text_when_no_tools_called() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("Hello world")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("hi").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty text response from provider"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Single tool call → final response
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_executes_single_tool_then_returns() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "echo".into(),
            arguments: r#"{"message": "hello from tool"}"#.into(),
            extra_content: None,
        }]),
        text_response("I ran the tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("run echo").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool execution"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Multi-step tool chain (tool A → tool B → response)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_multi_step_tool_chain() {
    let (counting_tool, count) = CountingTool::new();

    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "counter".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        tool_response(vec![ToolCall {
            id: "tc2".into(),
            name: "counter".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        tool_response(vec![ToolCall {
            id: "tc3".into(),
            name: "counter".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        text_response("Done after 3 calls"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(counting_tool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("count 3 times").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after multi-step chain"
    );
    assert_eq!(*count.lock().unwrap(), 3);
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Max-iteration checkpoint (resumable, not a hard bailout)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_emits_checkpoint_at_max_iterations() {
    // Create more tool calls than max_tool_iterations allows. Hitting the
    // cap must NOT error anymore: the harness emits a resumable checkpoint
    // and returns it Ok, so the transcript ends on a well-formed assistant
    // message instead of a dangling tool cycle that wedges the next turn
    // (bug-report-2026-05-26 A1). Every scripted response here is a tool
    // call, so the checkpoint summary call also yields no prose and the
    // deterministic fallback summary is used.
    let max_iters = 3;
    let mut responses = Vec::new();
    for i in 0..max_iters + 5 {
        responses.push(tool_response(vec![ToolCall {
            id: format!("tc{i}"),
            name: "echo".into(),
            // Vary the args each turn so the repeat-CALL breaker (which halts
            // identical (tool,args) loops) doesn't fire before the iteration
            // cap — this test exercises the max-iterations checkpoint path.
            arguments: format!(r#"{{"message": "loop {i}"}}"#),
            extra_content: None,
        }]));
    }

    let provider = Arc::new(ScriptedProvider::new(responses));

    let config = AgentConfig {
        max_tool_iterations: max_iters,
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, vec![Box::new(EchoTool)], config);

    let reply = agent
        .turn("infinite loop")
        .await
        .expect("hitting the iteration cap should return a checkpoint, not error");
    assert!(
        reply.contains("tool-call limit") && reply.contains("Next steps"),
        "Expected a resumable checkpoint summary, got: {reply}"
    );
    // The transcript ends on the assistant checkpoint (well-formed), which
    // is what lets the user's next message resume the task cleanly.
    assert!(
        matches!(
            agent.history().last(),
            Some(ConversationMessage::Chat(msg))
                if msg.role == "assistant" && msg.content.contains("Next steps")
        ),
        "history should end on the assistant checkpoint, got: {:?}",
        agent.history().last()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Unknown tool name recovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_unknown_tool_gracefully() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "nonexistent_tool".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        text_response("I couldn't find that tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("use nonexistent").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after unknown tool recovery"
    );

    // Verify the tool result named the unrecognized tool. Unknown-tool
    // recovery now flows through the tinyagents `UnknownToolPolicy::ReturnToolError`
    // path (issue #4249), which injects a `unknown tool `<name>` (arguments: …);
    // valid tools: [...]` result and continues so the model can self-correct.
    let has_tool_result = agent.history().iter().any(|msg| match msg {
        ConversationMessage::ToolResults(results) => results
            .iter()
            .any(|r| r.content.contains("unknown tool") && r.content.contains("nonexistent_tool")),
        _ => false,
    });
    assert!(
        has_tool_result,
        "Expected tool result naming the unknown tool"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Tool execution failure recovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_recovers_from_tool_failure() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "fail".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        text_response("Tool failed but I recovered"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(FailingTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("try failing tool").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool failure recovery"
    );
}

#[tokio::test]
async fn turn_recovers_from_tool_error() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "panicker".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        text_response("I recovered from the error"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(PanickingTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("try panicking").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool error recovery"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Provider error propagation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_propagates_provider_error() {
    let (mut agent, _tmp) = build_agent_with(
        Arc::new(FailingProvider),
        vec![],
        Box::new(NativeToolDispatcher),
    );

    let result = agent.turn("hello").await;
    assert!(result.is_err(), "Expected provider error to propagate");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. History trimming during long conversations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn history_trims_after_max_messages() {
    let max_history = 6;
    let mut responses = vec![];
    for _ in 0..max_history + 5 {
        responses.push(text_response("ok"));
    }

    let provider = Arc::new(ScriptedProvider::new(responses));
    let config = AgentConfig {
        max_history_messages: max_history,
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, vec![], config);

    for i in 0..max_history + 5 {
        let _ = agent.turn(&format!("msg {i}")).await.unwrap();
    }

    // System prompt (1) + trimmed messages
    // Should not exceed max_history + 1 (system prompt)
    assert!(
        agent.history().len() <= max_history + 1,
        "History length {} exceeds max {} + 1 (system)",
        agent.history().len(),
        max_history,
    );

    // System prompt should always be preserved
    let first = &agent.history()[0];
    assert!(matches!(first, ConversationMessage::Chat(c) if c.role == "system"));
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Memory auto-save round-trip
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auto_save_stores_messages_in_memory() {
    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response(
        "I remember everything",
    )]));

    let (mut agent, _tmp2) = build_agent_with_memory(
        provider,
        vec![],
        mem.clone(),
        true, // auto_save enabled
    );

    // Scoped like a real chat turn. The autosave only stores what a person sent
    // (`turn_origin::current_is_user_authored`), and production entry points
    // scope an origin — web chat `WebChat`, channels `ExternalChannel` — so a
    // test that skipped it would be asserting a shape no caller produces.
    let _ = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: "t-autosave".into(),
            client_id: "c-autosave".into(),
            request_id: None,
        },
        agent.turn("Remember this fact"),
    )
    .await
    .unwrap();

    // Both user message and assistant response should be saved. The assistant
    // reply is persisted synchronously, but the user message is saved
    // fire-and-forget (tokio::spawn in turn/core.rs, #3610), so it may land a
    // moment after `turn()` returns — poll briefly instead of reading once,
    // which otherwise races on a loaded CI runner under llvm-cov instrumentation.
    let mut count = 0;
    for _ in 0..50 {
        count = mem.count().await.unwrap();
        if count >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        count >= 2,
        "Expected at least 2 memory entries, got {count}"
    );
}

#[tokio::test]
async fn auto_save_disabled_does_not_store() {
    let (mem, _tmp) = make_sqlite_memory();
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("hello")]));

    let (mut agent, _tmp2) = build_agent_with_memory(
        provider,
        vec![],
        mem.clone(),
        false, // auto_save disabled
    );

    let _ = agent.turn("test message").await.unwrap();

    let count = mem.count().await.unwrap();
    assert_eq!(count, 0, "Expected 0 memory entries with auto_save off");
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Native vs XML dispatcher integration
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn xml_dispatcher_parses_and_loops() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        xml_tool_response("echo", r#"{"message": "xml-test"}"#),
        text_response("XML tool completed"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(XmlToolDispatcher),
    );

    let response = agent.turn("test xml").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response from XML dispatcher"
    );
}

#[tokio::test]
async fn native_dispatcher_sends_tool_specs() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let _ = agent.turn("hi").await.unwrap();

    // NativeToolDispatcher.should_send_tool_specs() returns true
    let dispatcher = NativeToolDispatcher;
    assert!(dispatcher.should_send_tool_specs());
}

#[tokio::test]
async fn xml_dispatcher_does_not_send_tool_specs() {
    let dispatcher = XmlToolDispatcher;
    assert!(!dispatcher.should_send_tool_specs());
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. Empty / whitespace-only LLM responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_errors_on_empty_text_response() {
    // A completion with no text *and* no tool calls is never a valid final
    // answer. The old behaviour returned `Ok("")`, which rendered as a blank
    // reply and silently wedged the thread; now it surfaces as a visible
    // error the user can retry on (bug-report-2026-05-26 A1).
    let provider = Arc::new(ScriptedProvider::new(vec![ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let err = agent
        .turn("hi")
        .await
        .expect_err("an empty provider response should surface as an error");
    assert!(
        err.to_string().contains("empty response"),
        "expected an empty-response error, got: {err}"
    );
}

#[tokio::test]
async fn turn_errors_on_none_text_response() {
    let provider = Arc::new(ScriptedProvider::new(vec![ChatResponse {
        text: None,
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let err = agent
        .turn("hi")
        .await
        .expect_err("a null-text provider response should surface as an error");
    assert!(
        err.to_string().contains("empty response"),
        "expected an empty-response error, got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. Mixed text + tool call responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_preserves_text_alongside_tool_calls() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        ChatResponse {
            text: Some("Let me check...".into()),
            tool_calls: vec![ToolCall {
                id: "tc1".into(),
                name: "echo".into(),
                arguments: r#"{"message": "hi"}"#.into(),
                extra_content: None,
            }],
            usage: None,
            reasoning_content: None,
        },
        text_response("Here are the results"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("check something").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty final response after mixed text+tool"
    );

    // The intermediate text should be preserved in history — either as a
    // standalone assistant `Chat` or carried on the `AssistantToolCalls` turn
    // that accompanied the tool call (the unified tinyagents representation
    // keeps the preface text on the tool-call turn).
    let has_intermediate = agent.history().iter().any(|msg| match msg {
        ConversationMessage::Chat(c) => c.role == "assistant" && c.content.contains("Let me check"),
        ConversationMessage::AssistantToolCalls { text, .. } => {
            text.as_deref().is_some_and(|t| t.contains("Let me check"))
        }
        _ => false,
    });
    assert!(has_intermediate, "Intermediate text should be in history");
}

// ═══════════════════════════════════════════════════════════════════════════
// 13. Multi-tool batch in a single response
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_multiple_tools_in_one_response() {
    let (counting_tool, count) = CountingTool::new();

    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_response(vec![
            ToolCall {
                id: "tc1".into(),
                name: "counter".into(),
                arguments: "{}".into(),
                extra_content: None,
            },
            ToolCall {
                id: "tc2".into(),
                name: "counter".into(),
                arguments: "{}".into(),
                extra_content: None,
            },
            ToolCall {
                id: "tc3".into(),
                name: "counter".into(),
                arguments: "{}".into(),
                extra_content: None,
            },
        ]),
        text_response("All 3 done"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(counting_tool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("batch").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after multi-tool batch"
    );
    assert_eq!(
        *count.lock().unwrap(),
        3,
        "All 3 tools should have been called"
    );
}

#[tokio::test]
async fn e2e_native_loop_executes_text_fallback_tool_calls_and_persists_history() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        ChatResponse {
            text: Some(
                "I'll inspect now.\n<invoke>{\"name\":\"echo\",\"arguments\":{\"message\":\"from-fallback\"}}</invoke>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        },
        text_response("Completed via tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("please use a tool").await.unwrap();
    assert_eq!(response, "Completed via tool");

    let mut assistant_tool_calls: Option<Vec<ToolCall>> = None;
    let mut tool_results: Option<Vec<ToolResultMessage>> = None;

    for msg in agent.history() {
        match msg {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
                assistant_tool_calls = Some(tool_calls.clone());
            }
            ConversationMessage::ToolResults(results) => {
                tool_results = Some(results.clone());
            }
            _ => {}
        }
    }

    let calls = assistant_tool_calls.expect("assistant tool calls should be persisted");
    let results = tool_results.expect("tool results should be persisted");
    assert_eq!(calls.len(), 1, "expected one parsed/persisted tool call");
    assert_eq!(results.len(), 1, "expected one tool result");
    assert_eq!(calls[0].name, "echo");
    assert!(
        calls[0].arguments.contains("from-fallback"),
        "persisted tool-call arguments should include fallback payload"
    );
    assert_eq!(
        calls[0].id, results[0].tool_call_id,
        "tool result must map to persisted assistant tool-call id"
    );
    assert_eq!(results[0].content, "from-fallback");
}

// ═══════════════════════════════════════════════════════════════════════════
// 14. System prompt generation & tool instructions
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn system_prompt_injected_on_first_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    assert!(agent.history().is_empty(), "History should start empty");

    let _ = agent.turn("hi").await.unwrap();

    // First message should be the system prompt
    let first = &agent.history()[0];
    assert!(
        matches!(first, ConversationMessage::Chat(c) if c.role == "system"),
        "First history entry should be system prompt"
    );
}
