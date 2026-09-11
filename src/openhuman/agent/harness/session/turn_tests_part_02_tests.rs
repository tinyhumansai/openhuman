use super::*;

/// The control case for the test above: with NO override the same agent offers
/// its tool, and the override state has reset after the previous turn — so the
/// suppression is genuinely one-shot.
#[tokio::test]
async fn turn_without_override_offers_tools_and_resets_after_a_suppressed_turn() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("first".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("second".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        Vec::new(),
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    // Turn 1: suppressed.
    agent.set_next_turn_overrides(crate::openhuman::agent::harness::session::TurnOverrides {
        suppress_tools: true,
        ..Default::default()
    });
    agent.turn("hi").await.expect("turn 1 succeeds");
    // Turn 2: no override set — the field must have reset to default.
    agent
        .turn("now do the work")
        .await
        .expect("turn 2 succeeds");

    let counts = provider_impl.tool_counts.lock().await;
    assert_eq!(counts.len(), 2);
    assert_eq!(counts[0], 0, "turn 1 suppressed its tools");
    assert!(
        counts[1] >= 1,
        "turn 2 must have the full toolbelt back (one-shot suppression, not a rebuild)"
    );
}

/// #1725: a per-turn `suppress_memory_agent` override skips the pre-turn
/// `agent_memory` retrieval even when the agent's policy is `Always`, so a
/// greeting is a SINGLE provider call with no "## Memory agent context" block.
#[tokio::test]
async fn turn_override_suppress_memory_agent_skips_memory_trigger() {
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("built-in agent definitions should load");

    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("parent final".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    let _workspace_env = WorkspaceEnvGuard::set(&workspace_path);
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    // The embedding seam, as above.
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(EchoTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .config(crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        })
        .workspace_dir(workspace_path)
        .auto_save(false)
        .event_context("turn-test-session", "turn-test-channel")
        .trigger_memory_agent(
            crate::openhuman::agent::harness::definition::TriggerMemoryAgent::Always,
        )
        .build()
        .unwrap();

    agent.set_next_turn_overrides(crate::openhuman::agent::harness::session::TurnOverrides {
        suppress_memory_agent: true,
        suppress_tools: true,
        suppress_active_goal: true,
        suppress_transcript_autoload: false,
    });
    let response = agent.turn("hi there").await.expect("turn should succeed");
    assert_eq!(response, "parent final");

    let requests = provider_impl.requests.lock().await;
    assert_eq!(
        requests.len(),
        1,
        "suppress_memory_agent must skip the memory-agent subagent call (only the parent turn runs)"
    );
    assert!(
        !requests[0]
            .iter()
            .any(|msg| msg.content.contains("## Memory agent context")),
        "no memory-agent context block may be injected on a suppressed turn"
    );
}

#[tokio::test]
async fn turn_uses_cached_transcript_prefix_on_first_iteration() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("cached-final".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        vec![],
        crate::openhuman::config::AgentConfig::default(),
        crate::openhuman::config::ContextConfig::default(),
    );
    agent.cached_transcript_messages = Some(vec![
        ChatMessage::system("cached-system"),
        ChatMessage::assistant("cached-assistant"),
    ]);

    let response = agent.turn("fresh").await.expect("turn should succeed");
    assert_eq!(response, "cached-final");
    assert!(agent.cached_transcript_messages.is_none());

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].len(), 3);
    assert_eq!(requests[0][0].content, "cached-system");
    assert_eq!(requests[0][1].content, "cached-assistant");
    assert_eq!(requests[0][2].role, "user");
    // #3602: every turn's user message is prefixed with the live
    // `Current Date & Time:` stamp, then the raw prompt. Assert the stamp
    // leads and the original prompt is preserved at the tail.
    assert!(
        requests[0][2].content.starts_with("Current Date & Time:"),
        "user message must lead with the per-turn time stamp: {}",
        requests[0][2].content
    );
    assert!(
        requests[0][2].content.ends_with("fresh"),
        "user message must preserve the original prompt: {}",
        requests[0][2].content
    );
}

/// Issue #4117 — a reply that already carries a well-formed leading block is
/// accepted verbatim: no corrective re-prompt, so exactly one provider call.
#[tokio::test]
async fn turn_accepts_valid_required_output_without_repair() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("{\"thoughts\": \"planning\", \"next_action\": \"answer\"}".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();

    let config = crate::openhuman::config::AgentConfig {
        max_tool_iterations: 3,
        max_history_messages: 10,
        required_output: Some(crate::openhuman::config::RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        }),
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

    assert!(
        response.contains("thoughts") && response.contains("next_action"),
        "a valid reply must be accepted verbatim, got: {response}"
    );
    // No corrective re-prompt fired — a single provider call.
    assert_eq!(provider_impl.requests.lock().await.len(), 1);
}

/// Issue #4117 — with no live progress sink (background/routing agent), when the
/// model emits prose without the mandated JSON block the turn engine re-prompts
/// and the recovered block-bearing reply *replaces* the omitting one.
#[tokio::test]
async fn turn_repairs_missing_required_output_via_reprompt() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Turn 1 final reply: prose only, no `thoughts` block.
            Ok(ChatResponse {
                text: Some("Sure, I'll handle that.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Corrective re-prompt: the model now emits a valid block.
            Ok(ChatResponse {
                text: Some(
                    "{\"thoughts\": \"planning the work\", \"next_action\": \"answer\"}".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();

    let config = crate::openhuman::config::AgentConfig {
        max_tool_iterations: 3,
        max_history_messages: 10,
        required_output: Some(crate::openhuman::config::RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        }),
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

    // The returned reply carries the recovered block.
    assert!(
        response.contains("thoughts") && response.contains("next_action"),
        "repaired reply must contain the required block, got: {response}"
    );
    // The omitting prose reply was re-prompted (2 provider calls total).
    assert_eq!(provider_impl.requests.lock().await.len(), 2);
    // History's trailing assistant message was rewritten to match.
    assert!(agent.history.iter().rev().any(|message| matches!(
        message,
        ConversationMessage::Chat(chat)
            if chat.role == "assistant" && chat.content.contains("next_action")
    )));
}

/// Issue #4117 — when the corrective re-prompt *also* omits the block (and no
/// live sink is attached), the turn engine synthesizes a minimal valid block and
/// prepends it to the original prose so the accepted turn leads with a
/// well-formed block and never loses the model's answer.
#[tokio::test]
async fn turn_synthesizes_required_output_when_reprompt_also_omits() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Turn 1 final reply: prose only.
            Ok(ChatResponse {
                text: Some("Working on it.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Re-prompt: still no block.
            Ok(ChatResponse {
                text: Some("Still just prose, sorry.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
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

    // The synthesized block is prepended to the ORIGINAL turn reply
    // ("Working on it."), not the failed corrective re-prompt — so the leading
    // JSON object carries the block and the original prose is preserved.
    let first_block = crate::openhuman::agent::harness::parse::extract_json_values(&response)
        .into_iter()
        .next();
    assert!(
        first_block
            .as_ref()
            .is_some_and(|v| v.get("thoughts").is_some()),
        "synthesized reply must lead with a `thoughts` block, got: {response}"
    );
    assert!(response.contains("Working on it."));
    assert_eq!(provider_impl.requests.lock().await.len(), 2);
}

/// Issue #4117 (the #4387 / sanil-23 streaming blocker) — when a live progress
/// sink is attached (the reply was streamed to the client) and the block is
/// omitted, the repair is **append-only**: the original prose is preserved, the
/// recovered block is *appended* after it (never replacing what the user saw),
/// and the appended block is streamed as a `TextDelta` continuation so the
/// returned reply is exactly the concatenation the client watched.
#[tokio::test]
async fn turn_appends_required_output_block_when_streamed_to_preserve_consistency() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Turn 1 final reply: prose only, streamed to the client.
            Ok(ChatResponse {
                text: Some("Sure, I'll handle that.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Corrective re-prompt: the model now emits a valid block.
            Ok(ChatResponse {
                text: Some(
                    "{\"thoughts\": \"planning the work\", \"next_action\": \"answer\"}".into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();

    let config = crate::openhuman::config::AgentConfig {
        max_tool_iterations: 3,
        max_history_messages: 10,
        required_output: Some(crate::openhuman::config::RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        }),
        ..crate::openhuman::config::AgentConfig::default()
    };

    let mut agent = make_agent_with_builder(
        provider,
        vec![],
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    // Attach a live progress sink and drain it concurrently so the streamed
    // deltas are captured without the bounded channel ever back-pressuring the
    // turn.
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let text_deltas = Arc::new(AsyncMutex::new(Vec::<String>::new()));
    let text_deltas_drain = text_deltas.clone();
    let drain = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let crate::openhuman::agent::progress::AgentProgress::TextDelta { delta, .. } =
                progress
            {
                text_deltas_drain.lock().await.push(delta);
            }
        }
    });
    agent.set_on_progress(Some(tx));

    let response = agent.turn("hello").await.expect("turn should succeed");

    // Drop the sink so the drain task observes channel close and finishes.
    agent.set_on_progress(None);
    // The continuation delta is sent (awaited) during the turn, so it is already
    // drained; guard the join with a timeout so a leaked sender clone can never
    // hang the test.
    let _ = timeout(Duration::from_secs(5), drain).await;

    // Append-only: the original prose is preserved AND precedes the block — the
    // streamed preview is a prefix of the final reply, never contradicted.
    assert!(
        response.contains("Sure, I'll handle that."),
        "original streamed prose must be preserved, not replaced: {response}"
    );
    assert!(
        response.contains("next_action"),
        "repaired reply must contain the required block: {response}"
    );
    let prose_at = response
        .find("Sure, I'll handle that.")
        .expect("prose present");
    let block_at = response.find("next_action").expect("block present");
    assert!(
        prose_at < block_at,
        "the block must be appended AFTER the already-streamed prose: {response}"
    );

    // The appended correction was streamed as a `TextDelta` continuation.
    let deltas = text_deltas.lock().await;
    assert!(
        deltas.iter().any(|d| d.contains("next_action")),
        "the appended block must be streamed as a TextDelta continuation, got: {deltas:?}"
    );
    assert_eq!(provider_impl.requests.lock().await.len(), 2);
}

/// Issue #4117 / #4900 — streamed path where the corrective re-prompt obeys
/// `repair_instruction` literally: it leads with the block **and continues with
/// the answer** (as the prompt asks). Because the original prose is already on
/// the client, only the block may be appended — appending the whole re-prompt
/// reply would duplicate the answer. Guards against that regression.
#[tokio::test]
async fn turn_appends_only_block_not_restated_answer_when_streamed() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            // Turn 1 final reply: prose only, streamed to the client.
            Ok(ChatResponse {
                text: Some("Paris is the capital of France.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Corrective re-prompt: block first, THEN the restated answer — exactly
            // what `repair_instruction` ("…then continue with your answer") elicits.
            Ok(ChatResponse {
                text: Some(
                    "{\"thoughts\": \"restating\", \"next_action\": \"answer\"}\n\n\
                     Paris is the capital of France."
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();

    let config = crate::openhuman::config::AgentConfig {
        max_tool_iterations: 3,
        max_history_messages: 10,
        required_output: Some(crate::openhuman::config::RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        }),
        ..crate::openhuman::config::AgentConfig::default()
    };

    let mut agent = make_agent_with_builder(
        provider,
        vec![],
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let text_deltas = Arc::new(AsyncMutex::new(Vec::<String>::new()));
    let text_deltas_drain = text_deltas.clone();
    let drain = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let crate::openhuman::agent::progress::AgentProgress::TextDelta { delta, .. } =
                progress
            {
                text_deltas_drain.lock().await.push(delta);
            }
        }
    });
    agent.set_on_progress(Some(tx));

    let response = agent
        .turn("what is the capital of France?")
        .await
        .expect("turn should succeed");

    agent.set_on_progress(None);
    let _ = timeout(Duration::from_secs(5), drain).await;

    // The block is appended and the original prose preserved …
    assert!(
        response.contains("next_action"),
        "repaired reply must contain the required block: {response}"
    );
    // … but the answer must appear exactly ONCE — the restated answer from the
    // re-prompt reply must not be appended after the block.
    assert_eq!(
        response.matches("Paris is the capital of France.").count(),
        1,
        "the answer must not be duplicated by appending the re-prompt's restated prose: {response}"
    );
    // Streamed continuation carried the block, not a second copy of the answer.
    let deltas = text_deltas.lock().await;
    let streamed_continuation: String = deltas.concat();
    assert!(
        streamed_continuation.contains("next_action"),
        "the appended block must be streamed as a continuation, got: {deltas:?}"
    );
    assert!(
        !streamed_continuation.contains("Paris is the capital of France."),
        "the streamed continuation must not restream the answer, got: {deltas:?}"
    );
}

/// Issue #4117 — streamed path, but the corrective re-prompt *also* omits the
/// block. A synthesized block is appended (never replacing the streamed prose)
/// and streamed as a continuation, so the accepted turn still leads with the
/// original prose and carries a well-formed block.
#[tokio::test]
async fn turn_appends_synthesized_block_when_streamed_reprompt_also_omits() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("Working on it.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            // Re-prompt: still no block.
            Ok(ChatResponse {
                text: Some("Still just prose, sorry.".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
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

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let text_deltas = Arc::new(AsyncMutex::new(Vec::<String>::new()));
    let text_deltas_drain = text_deltas.clone();
    let drain = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let crate::openhuman::agent::progress::AgentProgress::TextDelta { delta, .. } =
                progress
            {
                text_deltas_drain.lock().await.push(delta);
            }
        }
    });
    agent.set_on_progress(Some(tx));

    let response = agent.turn("hello").await.expect("turn should succeed");
    agent.set_on_progress(None);
    // The continuation delta is sent (awaited) during the turn, so it is already
    // drained; guard the join with a timeout so a leaked sender clone can never
    // hang the test.
    let _ = timeout(Duration::from_secs(5), drain).await;

    // Original prose preserved and leads; a synthesized `thoughts` block follows.
    assert!(response.contains("Working on it."));
    let prose_at = response.find("Working on it.").expect("prose present");
    let block_at = response.find("thoughts").expect("synth block present");
    assert!(
        prose_at < block_at,
        "synthesized block must be appended after the streamed prose: {response}"
    );
    let deltas = text_deltas.lock().await;
    assert!(
        deltas.iter().any(|d| d.contains("thoughts")),
        "the synthesized block must be streamed as a continuation, got: {deltas:?}"
    );
    assert_eq!(provider_impl.requests.lock().await.len(), 2);
}
