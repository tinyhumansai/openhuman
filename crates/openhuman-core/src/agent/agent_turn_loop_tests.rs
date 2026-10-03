use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// 1. Simple text response (no tools)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_returns_text_when_no_tools_called() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("Hello world")]));
    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

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
        tool_response(vec![NativeToolCall {
            id: "tc1".into(),
            name: "echo".into(),
            arguments: r#"{"message": "hello from tool"}"#.into(),
            extra_content: None,
        }]),
        text_response("I ran the tool"),
    ]));

    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

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
        tool_response(vec![NativeToolCall {
            id: "tc1".into(),
            name: "counter".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        tool_response(vec![NativeToolCall {
            id: "tc2".into(),
            name: "counter".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        tool_response(vec![NativeToolCall {
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
        Box::new(NativeDialect),
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
        responses.push(tool_response(vec![NativeToolCall {
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
        reply.contains("tool-call limit") && reply.contains("continue"),
        "Expected a resumable checkpoint summary, got: {reply}"
    );
    // The transcript ends on the assistant checkpoint (well-formed), which
    // is what lets the user's next message resume the task cleanly.
    assert!(
        matches!(
            agent.history().last(),
            Some(TranscriptEntry::Chat(msg))
                if msg.role.as_str() == "assistant" && msg.content.contains("tool-call limit")
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
        tool_response(vec![NativeToolCall {
            id: "tc1".into(),
            name: "nonexistent_tool".into(),
            arguments: "{}".into(),
            extra_content: None,
        }]),
        text_response("I couldn't find that tool"),
    ]));

    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

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
        TranscriptEntry::ToolResults(results) => results
            .iter()
            .any(|r| r.content.contains("unknown tool") && r.content.contains("nonexistent_tool")),
        TranscriptEntry::Chat(message) => {
            message.role.as_str() == "tool"
                && message.content.contains("unknown tool")
                && message.content.contains("nonexistent_tool")
        }
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
async fn turn_recovers_from_tool_failure_and_tool_error() {
    let cases: [(&str, Box<dyn Tool>); 2] = [
        ("fail", Box::new(FailingTool)),
        ("panicker", Box::new(PanickingTool)),
    ];
    for (tool_name, tool) in cases {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_response(vec![NativeToolCall {
                id: "tc1".into(),
                name: tool_name.into(),
                arguments: "{}".into(),
                extra_content: None,
            }]),
            text_response("I recovered"),
        ]));
        let (mut agent, _tmp) = build_agent_with(provider, vec![tool], Box::new(NativeDialect));

        let response = agent.turn("try the tool").await.unwrap();
        assert!(
            !response.is_empty(),
            "Expected non-empty response after {tool_name} recovery"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Provider error propagation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_propagates_provider_error() {
    let (mut agent, _tmp) =
        build_agent_with(Arc::new(FailingProvider), vec![], Box::new(NativeDialect));

    let result = agent.turn("hello").await;
    assert!(result.is_err(), "Expected provider error to propagate");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Long conversations keep their history (no message-count trim)
// ═══════════════════════════════════════════════════════════════════════════

/// Prompt-cache regression: the legacy `max_history_messages` bound used to
/// drop the oldest messages on every turn past it, which moved the head of the
/// provider's cached prompt prefix every turn (a full cache miss each time).
/// History is now bounded only by the token-aware context ladder, so a small
/// legacy bound is ignored and the message after the system prompt never moves.
#[tokio::test]
async fn history_is_not_trimmed_by_message_count() {
    let legacy_bound = 6;
    let turns = legacy_bound + 5;
    let responses = (0..turns).map(|_| text_response("ok")).collect();

    let provider = Arc::new(ScriptedProvider::new(responses));
    let config = AgentConfig {
        max_history_messages: legacy_bound,
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, vec![], config);

    for i in 0..turns {
        let _ = agent.turn(&format!("msg {i}")).await.unwrap();
    }

    let history = agent.history();
    // System prompt should always be preserved.
    assert!(matches!(&history[0], TranscriptEntry::Chat(c) if c.role.as_str() == "system"));
    // Every turn's user message is still there, and the first one still opens
    // the conversation right after the system prompt.
    assert!(
        history.len() > legacy_bound + 1,
        "history length {} was trimmed to the legacy bound {legacy_bound}",
        history.len()
    );
    let first_user = history
        .iter()
        .find_map(|m| match m {
            TranscriptEntry::Chat(c) if c.role.as_str() == "user" => Some(c.content.clone()),
            _ => None,
        })
        .expect("a user message");
    assert!(
        first_user.contains("msg 0"),
        "the oldest turn was dropped; first user message is {first_user:?}"
    );
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

    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(XmlDialect));

    let response = agent.turn("test xml").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response from XML dispatcher"
    );
}

#[tokio::test]
async fn native_dispatcher_sends_tool_specs() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

    let _ = agent.turn("hi").await.unwrap();

    // NativeDialect.should_send_tool_specs() returns true
    let dispatcher = NativeDialect;
    assert!(dispatcher.should_send_tool_specs());
}

#[tokio::test]
async fn xml_dispatcher_does_not_send_tool_specs() {
    let dispatcher = XmlDialect;
    assert!(!dispatcher.should_send_tool_specs());
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. Empty / whitespace-only LLM responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_errors_on_empty_or_null_text_response() {
    // A completion with no text *and* no tool calls is never a valid final
    // answer: it must surface as a visible error the user can retry on rather
    // than a blank reply (bug-report-2026-05-26 A1). The harness retries an
    // empty completion once, so both attempts are scripted empty.
    for text in [Some(String::new()), None] {
        let empty = || ChatResponse {
            text: text.clone(),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        };
        let provider = Arc::new(ScriptedProvider::new(vec![empty(), empty()]));
        let script = Arc::clone(&provider);

        let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeDialect));

        let reply = agent
            .turn("hi")
            .await
            .expect_err("an empty provider response must error");
        assert_eq!(
            script.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the empty completion must be retried exactly once before erroring ({text:?})"
        );
        assert!(
            reply.to_string().contains("empty response"),
            "expected a deterministic empty-response close ({text:?}), got: {reply}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. Mixed text + tool call responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_preserves_text_alongside_tool_calls() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        ChatResponse {
            text: Some("Let me check...".into()),
            tool_calls: vec![NativeToolCall {
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

    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

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
        TranscriptEntry::Chat(c) => {
            c.role.as_str() == "assistant" && c.content.contains("Let me check")
        }
        TranscriptEntry::AssistantToolCalls { text, .. } => {
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
            NativeToolCall {
                id: "tc1".into(),
                name: "counter".into(),
                arguments: "{}".into(),
                extra_content: None,
            },
            NativeToolCall {
                id: "tc2".into(),
                name: "counter".into(),
                arguments: "{}".into(),
                extra_content: None,
            },
            NativeToolCall {
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
        Box::new(NativeDialect),
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

    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

    let response = agent.turn("please use a tool").await.unwrap();
    assert_eq!(response, "Completed via tool");

    let history = agent.history();
    let has_assistant_call = history.iter().any(|message| match message {
        TranscriptEntry::AssistantToolCalls { tool_calls, .. } => tool_calls
            .iter()
            .any(|call| call.name == "echo" && call.arguments.contains("from-fallback")),
        TranscriptEntry::Chat(message)
            if message.role.as_str() == "assistant"
                && message.content.contains("\"tool_calls\"")
                && message.content.contains("\"echo\"") =>
        {
            message.content.contains("from-fallback")
        }
        _ => false,
    });
    let has_tool_result = history.iter().any(|message| match message {
        TranscriptEntry::ToolResults(results) => results
            .iter()
            .any(|result| result.content.contains("from-fallback")),
        TranscriptEntry::Chat(message) => {
            message.role.as_str() == "tool" && message.content.contains("from-fallback")
        }
        _ => false,
    });
    assert!(
        has_assistant_call,
        "assistant tool call should be persisted"
    );
    assert!(has_tool_result, "tool result should be persisted");
}

// ═══════════════════════════════════════════════════════════════════════════
// 14. System prompt generation & tool instructions
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn system_prompt_injected_on_first_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) =
        build_agent_with(provider, vec![Box::new(EchoTool)], Box::new(NativeDialect));

    assert!(agent.history().is_empty(), "History should start empty");

    let _ = agent.turn("hi").await.unwrap();

    // First message should be the system prompt
    let first = &agent.history()[0];
    assert!(
        matches!(first, TranscriptEntry::Chat(c) if c.role.as_str() == "system"),
        "First history entry should be system prompt"
    );
}
