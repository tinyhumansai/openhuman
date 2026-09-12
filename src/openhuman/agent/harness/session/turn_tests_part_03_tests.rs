use super::*;

/// Issue #4117 — when the corrective re-prompt call itself fails, enforcement
/// still guarantees a well-formed block: the deterministic synthesized fallback
/// is used so the turn is never left without one (no live sink → replace path).
#[tokio::test]
async fn turn_synthesizes_required_output_when_reprompt_call_fails() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("Working on it.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Re-prompt call errors out.
            Err(anyhow::anyhow!("provider boom")),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();

    let config = crate::openhuman::config::AgentConfig {
        max_tool_iterations: 3,
        max_history_messages: 10,
        required_output: Some(crate::openhuman::config::RequiredOutputContract::new(
            "thoughts",
        )),
        ..crate::openhuman::config::AgentConfig::default()
    };

    let mut agent = make_agent_with_builder(
        provider,
        vec![],
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    let response = agent.turn("hello").await.expect("turn should succeed");

    // Deterministic fallback: a synthesized block leads, original prose kept.
    let first_block = crate::openhuman::agent::harness::parse::extract_json_values(&response)
        .into_iter()
        .next();
    assert!(
        first_block
            .as_ref()
            .is_some_and(|v| v.get("thoughts").is_some()),
        "failed re-prompt must still yield a synthesized leading block, got: {response}"
    );
    assert!(response.contains("Working on it."));
}

#[tokio::test]
async fn turn_emits_checkpoint_when_max_tool_iterations_are_exceeded() {
    // First response forces a tool call (consuming the single allowed
    // iteration); the second is the model-written checkpoint the harness
    // requests (tools disabled) once the cap is hit. The turn must NOT
    // error anymore — it returns a resumable checkpoint so the thread stays
    // well-formed and the user can continue on their next message
    // (bug-report-2026-05-26 A1).
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some(
                    "**Done so far:** ran echo.\n**Next steps:** I'll continue from here.".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("turn should emit a checkpoint at the iteration cap, not error");
    assert!(
        reply.contains("Next steps"),
        "checkpoint should summarize next steps, got: {reply}"
    );
    // The tool-call history from the capped iteration is preserved...
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls { tool_calls, .. } if tool_calls.len() == 1
    )));
    // ...and the transcript ends on a well-formed assistant message (the
    // checkpoint), never a dangling tool cycle — this is what stops the
    // next message from silently wedging the thread.
    assert!(
        matches!(
            agent.history.last(),
            Some(ConversationMessage::Chat(msg))
                if msg.role == "assistant" && msg.content.contains("Next steps")
        ),
        "history should end on the assistant checkpoint, got: {:?}",
        agent.history.last()
    );
}

#[tokio::test]
async fn turn_errors_on_empty_provider_response() {
    // A completion with no text and no tool calls is never a valid final
    // answer — surface it as an error instead of accepting a blank reply,
    // which previously rendered as silence and wedged the thread
    // (bug-report-2026-05-26 A1, defect B).
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![],
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );

    let err = agent
        .turn("hello")
        .await
        .expect_err("an empty provider response should surface as an error");
    assert!(
        err.to_string().contains("empty response"),
        "expected an empty-response error, got: {err}"
    );
}

#[tokio::test]
async fn turn_checkpoint_falls_back_to_deterministic_summary_when_model_summary_empty() {
    // Tool call consumes the single iteration; the checkpoint request then
    // comes back empty. The harness must fall back to a deterministic
    // done/next summary so the turn never returns blank — the safety net
    // that guarantees the thread can't re-wedge (bug-report-2026-05-26 A1).
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("empty model checkpoint should fall back, not error");
    assert!(
        reply.contains("tool-call limit"),
        "deterministic fallback summary expected, got: {reply}"
    );
    assert!(
        reply.contains("echo"),
        "fallback should list the tool that ran, got: {reply}"
    );
}

#[tokio::test]
async fn turn_checkpoint_rejects_pformat_wrapup_without_streaming_it() {
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                ..ChatResponse::default()
            }),
            Ok(ChatResponse {
                text: Some("<tool_call>echo[]</tool_call>".into()),
                ..ChatResponse::default()
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let registry = crate::openhuman::agent::pformat::build_registry(&tools);
    let mut agent = make_agent_with_builder_and_dispatcher(
        provider,
        tools,
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
        Box::new(PFormatToolDispatcher::new(registry)),
    );
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
    agent.set_on_progress(Some(progress_tx));

    let reply = agent
        .turn("hello")
        .await
        .expect("P-Format wrap-up call should use the deterministic checkpoint");
    assert!(
        reply.contains("tool-call limit"),
        "P-Format wrap-up must be rejected, got: {reply}"
    );

    agent.set_on_progress(None);
    let mut rendered_invalid_wrapup = false;
    while let Ok(progress) = progress_rx.try_recv() {
        if let crate::openhuman::agent::progress::AgentProgress::TextDelta { delta, .. } = progress
        {
            rendered_invalid_wrapup |= delta.contains("echo[]");
        }
    }
    assert!(
        !rendered_invalid_wrapup,
        "rejected P-Format wrap-up must not be emitted to the progress sink"
    );
}

#[tokio::test]
async fn turn_synthesizes_final_answer_when_tool_turn_yields_no_text() {
    // #4093: the model runs a tool and then yields a terminating response with
    // NO text and NO further tool calls — the turn did work but would end
    // silently. Because the cap was not hit, this is not a checkpoint case; the
    // harness must enforce the "must produce a final response" terminal step by
    // re-prompting the model (tools disabled) for a closing summary and
    // returning that instead of a blank reply.
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Tool iteration (well under the cap).
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Terminal response with no text and no tool calls — the silent end.
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // The harness's forced final-answer re-prompt (tools disabled).
            Ok(ChatResponse {
                text: Some("All done — I ran echo and it succeeded.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 5,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("a tool-only turn with no final text should synthesize one, not error");
    assert!(
        !reply.trim().is_empty(),
        "turn must never end with an empty final message (#4093), got: {reply:?}"
    );
    assert!(
        reply.contains("I ran echo"),
        "the synthesized final message should be the model's wrap-up, got: {reply}"
    );
    // The transcript must end on the assistant's final message, not a dangling
    // tool cycle.
    assert!(
        matches!(
            agent.history.last(),
            Some(ConversationMessage::Chat(msg))
                if msg.role == "assistant" && !msg.content.trim().is_empty()
        ),
        "history should end on a non-empty assistant message, got: {:?}",
        agent.history.last()
    );
    // ...and the blank terminal assistant response (folded in from the turn
    // outcome) must have been dropped, not left dangling before the synthesized
    // answer (Codex review).
    assert!(
        !agent.history.iter().any(|m| matches!(
            m,
            ConversationMessage::Chat(msg)
                if msg.role == "assistant" && msg.content.trim().is_empty()
        )),
        "no blank assistant turn should remain in history, got: {:?}",
        agent.history
    );
}

#[tokio::test]
async fn turn_final_answer_falls_back_to_deterministic_summary_when_reprompt_empty() {
    // #4093 safety net: the tool ran, the model yielded no final text, and the
    // forced final-answer re-prompt ALSO came back empty. The harness must fall
    // back to a deterministic summary of the tool calls so the turn is never
    // blank — and, unlike the cap path, it must read as a completed summary
    // rather than a paused "tool-call limit" checkpoint.
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Re-prompt for a final answer also returns empty.
            Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 5,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("hello")
        .await
        .expect("empty final re-prompt should fall back deterministically, not error");
    assert!(
        !reply.trim().is_empty(),
        "deterministic fallback must be non-empty (#4093), got: {reply:?}"
    );
    assert!(
        reply.contains("echo"),
        "fallback should list the tool that ran, got: {reply}"
    );
    assert!(
        !reply.contains("tool-call limit"),
        "a non-capped turn must not claim it hit the tool-call limit, got: {reply}"
    );
    // The blank terminal assistant response must not linger before the
    // deterministic summary (Codex review).
    assert!(
        !agent.history.iter().any(|m| matches!(
            m,
            ConversationMessage::Chat(msg)
                if msg.role == "assistant" && msg.content.trim().is_empty()
        )),
        "no blank assistant turn should remain in history, got: {:?}",
        agent.history
    );
}

#[tokio::test]
async fn summarize_turn_wrapup_rejects_prompt_tool_call_and_preserves_usage() {
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
            tool_calls: vec![],
            usage: Some(UsageInfo {
                input_tokens: 13,
                output_tokens: 5,
                cached_input_tokens: 3,
                charged_amount_usd: 0.07,
                ..UsageInfo::default()
            }),
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let agent = make_agent_with_builder(
        provider,
        vec![],
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );

    let (summary, usage) = agent
        .summarize_turn_wrapup(&[], "test-model", 1, "write a wrap-up")
        .await;

    assert!(
        summary.is_empty(),
        "prompt-formatted tool calls must trigger the deterministic fallback"
    );
    let usage = usage.expect("rejected wrap-up must preserve provider usage");
    assert_eq!(usage.input_tokens, 13);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.cached_input_tokens, 3);
    assert_eq!(usage.charged_amount_usd, 0.07);
}

#[tokio::test]
async fn turn_checkpoint_usage_is_folded_into_transcript_accounting() {
    // The extra checkpoint provider call costs tokens; those must land in
    // the persisted transcript's cumulative accounting rather than being
    // silently dropped (CodeRabbit review on bug-report-2026-05-26 A1).
    let provider: Arc<dyn ChatModel<()>> = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Tool iteration — provider reports no usage.
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Checkpoint call — reports usage that must be accounted for.
            Ok(ChatResponse {
                text: Some("**Done so far:** ran echo.\n**Next steps:** continue.".into()),
                tool_calls: vec![],
                usage: Some(UsageInfo {
                    input_tokens: 11,
                    output_tokens: 4,
                    cached_input_tokens: 2,
                    charged_amount_usd: 0.05,
                    ..UsageInfo::default()
                }),
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 1,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    agent
        .turn("hello")
        .await
        .expect("turn should emit a checkpoint at the iteration cap");

    let transcript = transcript::read_transcript(
        agent
            .session_transcript_path
            .as_ref()
            .expect("checkpoint turn should persist a transcript"),
    )
    .expect("transcript should be readable");
    // Only the checkpoint call reported usage, so the turn totals must equal
    // exactly its numbers — proof the extra call is accounted for, not lost.
    assert_eq!(
        transcript.meta.input_tokens, 11,
        "checkpoint input tokens should be folded into the turn total"
    );
    assert_eq!(transcript.meta.output_tokens, 4);
    assert_eq!(transcript.meta.cached_input_tokens, 2);
}

#[tokio::test]
async fn dedicated_profile_experience_recall_merges_shared_legacy_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dedicated = make_real_memory(&tmp.path().join("dedicated"));
    let shared = make_real_memory(&tmp.path().join("shared"));
    AgentExperienceStore::new(shared.clone())
        .put(AgentExperience {
            id: "legacy-shared-deploy".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source: ExperienceSource::ToolLoop,
            agent_id: None,
            entrypoint: None,
            profile_id: None,
            task_fingerprint: "deploy-rust-service".into(),
            task_summary: "Deploy the Rust service safely".into(),
            tools_used: vec![],
            tool_sequence: vec![],
            outcome: ExperienceOutcome::Success,
            error_class: None,
            lesson: "Legacy shared deployment guidance".into(),
            reuse_hint: "Check the release health endpoint".into(),
            avoid_hint: None,
            confidence: 0.9,
            tags: vec![],
            payload_hash: None,
            dismissed: false,
        })
        .await
        .unwrap();

    let agent = Agent::builder()
        .chat_model(Arc::new(DummyProvider))
        .tools(vec![])
        .memory(dedicated)
        .shared_experience_memory(Some(shared))
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(tmp.path().to_path_buf())
        .event_context("profile-experience-test", "web_chat")
        .active_profile_id(Some("alice".into()))
        .profile_memory_storage("memory-alice".into(), "session_raw-alice".into())
        .learning_enabled(true)
        .build()
        .unwrap();

    let enriched = agent
        .inject_agent_experience_context(
            "How should I deploy the Rust service?",
            "original prompt".into(),
        )
        .await;

    assert!(enriched.contains("Legacy shared deployment guidance"));
    assert!(enriched.contains("original prompt"));
}

#[tokio::test]
async fn fetch_learned_context_returns_empty_when_both_flags_off() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem = make_real_memory(tmp.path());

    // Store a pinned preference so we can verify it is NOT returned.
    mem.store(
        "user_profile",
        "pinned/tooling/package_manager",
        "[pinned] (class=tooling) package_manager: pnpm",
        crate::openhuman::memory::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let agent = make_agent_with_memory(
        mem,
        tmp.path().to_path_buf(),
        false, // learning_enabled
        false, // explicit_preferences_enabled
    );

    let learned = agent.fetch_learned_context().await;

    assert!(
        learned.user_profile.is_empty(),
        "both flags off: user_profile must be empty, got {:?}",
        learned.user_profile
    );
    assert!(learned.observations.is_empty());
    assert!(learned.patterns.is_empty());
    assert!(learned.reflections.is_empty());
}

/// Issue #6014: a capped turn concludes from **inside** the loop.
///
/// The turn used to exit on the cap and then make a second, out-of-band call
/// asking for a checkpoint. That call ran with none of the loop's context
/// management and failed silently into a digest of tool names — on the very
/// turns that had gathered the most. The last permitted model call now does the
/// concluding itself: tools withdrawn, wrap-up instruction appended, answer
/// produced by an ordinary loop iteration.
///
/// Pinned here: the tool schemas really are withdrawn on that call (not merely
/// discouraged in prose), the loop's own text becomes the reply, the turn still
/// reports as capped, and — the whole point — no extra provider call is made.
#[tokio::test]
async fn a_capped_turn_concludes_inside_the_loop_without_a_second_call() {
    let recorded = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Call 1 spends the turn's only tool round.
            Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Call 2 is the last permitted one, so the middleware turns it into
            // the conclusion. A third response is deliberately NOT scripted: if
            // anything still made an out-of-band wrap-up call, the provider
            // would run dry and the turn would fail.
            Ok(ChatResponse {
                text: Some("The echo tool returned `hello`, which answers the question.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = recorded.clone();

    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig {
            // Two, not one: a single-call budget would make the FIRST call the
            // concluding one, so the turn could never run a tool at all — which
            // the middleware treats as a misconfiguration and declines to touch.
            max_tool_iterations: 2,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let reply = agent
        .turn("what does echo say?")
        .await
        .expect("a capped turn should conclude, not error");

    // The loop's own final text is the reply — it reports the RESULT, which is
    // what the old process-shaped checkpoint could not do.
    assert!(
        reply.contains("returned `hello`"),
        "the in-loop conclusion should be the reply, got: {reply}"
    );

    let tool_counts = recorded.tool_counts.lock().await.clone();
    assert_eq!(
        tool_counts.len(),
        2,
        "exactly two provider calls: the tool round and the conclusion — an out-of-band wrap-up \
         would make a third, {tool_counts:?}"
    );
    assert!(
        tool_counts[0] > 0,
        "the first call must be offered the tool belt, got {tool_counts:?}"
    );
    assert_eq!(
        tool_counts[1], 0,
        "the concluding call must be offered NO tools — the instruction alone is a request the \
         model may ignore, {tool_counts:?}"
    );

    // Still reported as a pause, so callers that render a "continue" affordance
    // (and the workflow runner, which must not settle a capped node as
    // finished) keep the signal they read today.
    assert!(
        agent.last_turn_hit_cap(),
        "a turn that concluded at its cap is still a capped turn"
    );

    // The transcript ends on that conclusion exactly once: it came from the
    // loop, so it is already folded in via the run's conversation, and the cap
    // branch must not append a second copy.
    let conclusions = agent
        .history
        .iter()
        .filter(|message| {
            matches!(message, ConversationMessage::Chat(msg)
                if msg.role == "assistant" && msg.content.contains("returned `hello`"))
        })
        .count();
    assert_eq!(
        conclusions, 1,
        "the conclusion should appear once in history, not be re-pushed by the cap branch"
    );
}
