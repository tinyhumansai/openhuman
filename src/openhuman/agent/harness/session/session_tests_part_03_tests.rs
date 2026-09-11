use super::*;

/// Cold-boot web-chat resume must prefer the full-fidelity `session_raw`
/// transcript over lossy conversation-log prose. This is the regression for the
/// "model forgets its tool interactions after an app restart" bug: once the
/// in-memory agent is dropped and a fresh agent cold-boots for the same thread,
/// the resumed context must still carry the tool call, the tool-role result, and
/// the reasoning that prose seeding (`seed_resume_from_messages`) discards.
#[test]
fn seed_resume_from_thread_transcript_preserves_tool_calls_and_reasoning() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    use super::super::transcript::{self, MessageUsage, TranscriptMeta, TurnUsage};
    use crate::openhuman::agent::messages::ChatMessage;
    use crate::openhuman::inference::provider::ToolCall;

    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    let thread_id = "thr_resume_fidelity";

    // ── Simulate a prior session persisted to session_raw carrying a tool
    // call + reasoning on the tool-calling assistant turn and a tool-role
    // result — exactly the fidelity the prose fallback drops. ──
    let mut assistant_toolcall = ChatMessage::assistant("Let me look that up.");
    transcript::attach_turn_usage_metadata(
        &mut assistant_toolcall,
        &TurnUsage {
            provider: "openai".to_string(),
            model: "gpt-x".to_string(),
            usage: MessageUsage {
                input: 10,
                output: 5,
                cached_input: 0,
                context_window: 0,
                cost_usd: 0.0,
            },
            ts: "2026-01-01T00:00:00Z".to_string(),
            reasoning_content: Some("I should search the web for the price.".to_string()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: r#"{"query":"btc price"}"#.to_string(),
                extra_content: None,
            }],
            iteration: 1,
        },
    );

    let messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("what is btc price"),
        assistant_toolcall,
        ChatMessage::tool(r#"{"tool_call_id":"call_1","content":"$80,000"}"#),
        ChatMessage::assistant("BTC is around $80,000."),
    ];
    let meta = TranscriptMeta {
        agent_name: "orchestrator_thread-resume".to_string(),
        agent_id: Some("orchestrator".to_string()),
        agent_type: Some("root".to_string()),
        dispatcher: "native".to_string(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".to_string(),
        updated: "2026-01-01T00:00:00Z".to_string(),
        turn_count: 1,
        input_tokens: 10,
        output_tokens: 5,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
    };
    // Root stem: no `__`, so `find_root_transcript_for_thread` accepts it.
    let path = transcript::resolve_keyed_transcript_path(&wsp, "1700000000_orchestrator")
        .expect("resolve transcript path");
    transcript::write_transcript(&path, &messages, &meta, None).expect("write transcript");

    // ── Cold boot: a brand-new agent for the same thread whose agent
    // definition name deliberately does NOT match the transcript stem — the
    // resume must route purely by thread id, not by agent name. ──
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();
    let mut agent = Agent::builder()
        .chat_model(Arc::new(MockProvider {
            responses: Mutex::new(vec![]),
        }))
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .agent_definition_name("some_other_agent_name")
        .workspace_dir(wsp.clone())
        .build()
        .expect("agent build should succeed");

    let loaded = agent.seed_resume_from_thread_transcript(thread_id);
    assert!(
        loaded,
        "cold-boot resume must load the thread's root transcript"
    );

    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cached transcript populated");

    // The tool-role result must survive — prose seeding would have dropped it.
    assert!(
        cached.iter().any(|m| m.role == "tool"),
        "resumed context must include the tool-role result message"
    );

    // The assistant tool call + reasoning survive, carried in metadata.
    let tool_call_carrier = cached
        .iter()
        .find(|m| {
            m.role == "assistant"
                && m.extra_metadata
                    .as_ref()
                    .and_then(|v| v.get("openhuman_turn_usage"))
                    .is_some()
        })
        .expect("resumed context must include the assistant tool-call turn");
    let usage_value = tool_call_carrier
        .extra_metadata
        .as_ref()
        .and_then(|v| v.get("openhuman_turn_usage"))
        .cloned()
        .expect("turn usage metadata present");
    let parsed: TurnUsage = serde_json::from_value(usage_value).expect("turn usage deserializes");
    assert!(
        parsed.tool_calls.iter().any(|c| c.name == "web_search"),
        "the persisted tool call must round-trip into the resumed context"
    );
    assert_eq!(
        parsed.reasoning_content.as_deref(),
        Some("I should search the web for the price."),
        "reasoning content must be preserved on resume"
    );
}

/// Cold-boot resume over an **append-only** transcript that carries a
/// compaction record: the resumed model context must equal the REDUCED set the
/// compaction installed (byte-identical to what the old full-rewrite produced),
/// not the full pre-compaction history.
#[test]
fn seed_resume_replays_compaction_to_reduced_context() {
    use super::super::transcript::{self, TranscriptMeta};
    use crate::openhuman::agent::messages::ChatMessage;

    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    let thread_id = "thr_compaction_resume";

    let meta = TranscriptMeta {
        agent_name: "orchestrator_thread-compact".to_string(),
        agent_id: Some("orchestrator".to_string()),
        agent_type: Some("root".to_string()),
        dispatcher: "native".to_string(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".to_string(),
        updated: "2026-01-01T00:00:00Z".to_string(),
        turn_count: 2,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
    };
    let path = transcript::resolve_keyed_transcript_path(&wsp, "1700000000_orchestrator")
        .expect("resolve transcript path");

    // Turn 1: a full exchange. Turn 2: a context reduction (not a prefix) that
    // must land as a compaction record.
    let full = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("q1"),
        ChatMessage::assistant("a1"),
        ChatMessage::user("q2"),
        ChatMessage::assistant("a2"),
    ];
    transcript::append_transcript_turn(&path, &[], &full, &meta, None, None)
        .expect("append turn 1");
    let reduced = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::assistant("[summary] q1/q2"),
        ChatMessage::user("q3"),
        ChatMessage::assistant("a3"),
    ];
    transcript::append_transcript_turn(&path, &full, &reduced, &meta, None, None)
        .expect("append turn 2 (compaction)");

    let mut agent = build_minimal_agent_with_definition_name(Some("some_other_agent_name"));
    agent.workspace_dir = wsp.clone();

    let loaded = agent.seed_resume_from_thread_transcript(thread_id);
    assert!(
        loaded,
        "cold-boot resume must load the compacted transcript"
    );
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cached transcript populated");
    assert_eq!(
        cached
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["system prompt", "[summary] q1/q2", "q3", "a3"],
        "resumed context must be the reduced set the compaction installed"
    );
}

/// #5351: a profile-scoped session (running in its own `session_raw-<id>/`
/// subtree) must still resume a thread whose earlier turns were written under a
/// DIFFERENT profile's subtree — here the shared `session_raw/`. Without the
/// cross-dir fallback the Reasoning profile could not see the plan the Quick
/// profile wrote, dropping all prior context on a mid-thread Quick↔Reasoning
/// switch.
#[test]
fn seed_resume_from_thread_transcript_crosses_profile_scoped_dirs() {
    use super::super::transcript::{self, TranscriptMeta};
    use crate::openhuman::agent::messages::ChatMessage;

    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    let thread_id = "thr_cross_profile";

    // Prior turns written by the QUICK (default) profile into the SHARED
    // `session_raw/` subtree.
    let messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("set up Minimax for image generation"),
        ChatMessage::assistant("Minimax is configured as the image generator."),
    ];
    let meta = TranscriptMeta {
        agent_name: "orchestrator_thr_cross_pr".to_string(),
        agent_id: Some("orchestrator".to_string()),
        agent_type: Some("root".to_string()),
        dispatcher: "native".to_string(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".to_string(),
        updated: "2026-01-01T00:00:00Z".to_string(),
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
    };
    // The shared `session_raw/` (default resolve path) — the Quick profile's dir.
    let path = transcript::resolve_keyed_transcript_path(&wsp, "1700000000_orchestrator")
        .expect("resolve transcript path");
    transcript::write_transcript(&path, &messages, &meta, None).expect("write transcript");

    // The REASONING profile runs in a scoped `session_raw-1/` subtree — its own
    // dir holds no transcript for this thread, so the in-dir lookup misses and
    // only the cross-dir fallback can recover the conversation.
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.workspace_dir = wsp.clone();
    agent.session_raw_subdir = "session_raw-1".to_string();

    let loaded = agent.seed_resume_from_thread_transcript(thread_id);
    assert!(
        loaded,
        "a profile-scoped session must resume the thread's transcript from the shared \
         session_raw dir via the cross-dir fallback (#5351)"
    );
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cached transcript populated");
    assert!(
        cached.iter().any(|m| m.content.contains("image generator")),
        "the prior plan context must be recovered across the profile-scoped dir boundary"
    );
}

/// #5351 regression guard: resume must pick the NEWEST transcript across profile
/// dirs, never the one in the agent's own dir. After the Reasoning profile is
/// healed back to the shared `session_raw/`, an OLDER transcript there must not
/// shadow the NEWER turns the profile wrote into its (pre-heal) scoped
/// `session_raw-1/` — otherwise the switch drops the most recent context and the
/// seeded history diverges from what the transcript view shows.
#[test]
fn seed_resume_from_thread_transcript_picks_newest_across_profile_dirs() {
    use super::super::transcript::{self, TranscriptMeta};
    use crate::openhuman::agent::messages::ChatMessage;

    let ws = tempfile::TempDir::new().expect("temp workspace");
    let wsp = ws.path().to_path_buf();
    let thread_id = "thr_newest_wins";

    let meta = |stamp: &str| TranscriptMeta {
        agent_name: "orchestrator_thr_newest".to_string(),
        agent_id: Some("orchestrator".to_string()),
        agent_type: Some("root".to_string()),
        dispatcher: "native".to_string(),
        provider: None,
        model: None,
        created: stamp.to_string(),
        updated: stamp.to_string(),
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread_id.to_string()),
        task_id: None,
    };

    // OLDER transcript in the agent's OWN (shared) dir.
    let older = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("draft plan"),
        ChatMessage::assistant("early draft, details TBD"),
    ];
    let old_path = wsp
        .join("session_raw")
        .join("1700000000_orchestrator.jsonl");
    std::fs::create_dir_all(old_path.parent().unwrap()).unwrap();
    transcript::write_transcript(&old_path, &older, &meta("2026-01-01T00:00:00Z"), None)
        .expect("write older");

    // NEWER transcript in a sibling scoped dir (written pre-heal, higher stem).
    let newer = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("finalize plan"),
        ChatMessage::assistant("FINAL: Minimax is the image generator"),
    ];
    let new_path = wsp
        .join("session_raw-1")
        .join("1700009999_orchestrator.jsonl");
    std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
    transcript::write_transcript(&new_path, &newer, &meta("2026-02-02T00:00:00Z"), None)
        .expect("write newer");

    // Agent runs in the shared dir (healed). Own-dir-first would wrongly pick the
    // older draft; newest-across-dirs must pick the finalized plan.
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.workspace_dir = wsp.clone();
    agent.session_raw_subdir = "session_raw".to_string();

    assert!(agent.seed_resume_from_thread_transcript(thread_id));
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cached transcript populated");
    assert!(
        cached.iter().any(|m| m.content.contains("FINAL")),
        "resume must load the NEWEST transcript across profile dirs, not the older own-dir copy"
    );
    assert!(
        !cached.iter().any(|m| m.content.contains("early draft")),
        "the older own-dir transcript must not shadow the newer sibling"
    );
}

/// When no root transcript exists for the thread, the transcript resume is a
/// no-op returning `false` so the caller falls back to prose-pair seeding.
#[test]
fn seed_resume_from_thread_transcript_returns_false_without_transcript() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    assert!(!agent.seed_resume_from_thread_transcript("thr_missing"));
    assert!(agent.cached_transcript_messages.is_none());
}

/// Transcript resume must not stomp an already-warm agent (in-process session
/// cache hit) — mirrors the `seed_resume_from_messages` warm-agent guard.
#[test]
fn seed_resume_from_thread_transcript_is_noop_on_warm_agent() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.cached_transcript_messages = Some(vec![
        crate::openhuman::agent::messages::ChatMessage::system("warm prefix"),
    ]);
    assert!(!agent.seed_resume_from_thread_transcript("thr_x"));
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("still populated");
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].content, "warm prefix");
}

/// `hide_tools` on an agent that already has a visible-tool filter must drop
/// only the named tools and leave the rest of the belt intact.
#[test]
fn hide_tools_drops_named_from_existing_filter() {
    let mut agent = build_minimal_agent_with_definition_name(None);
    agent.set_visible_tool_names(
        ["alpha".to_string(), "beta".to_string(), "echo".to_string()]
            .into_iter()
            .collect(),
    );

    agent.hide_tools(&["echo"]);

    let visible = agent.visible_tool_names_for_test();
    assert!(visible.contains("alpha") && visible.contains("beta"));
    assert!(
        !visible.contains("echo"),
        "hidden tool must be removed from the existing filter; visible = {visible:?}"
    );
}

/// `hide_tools` on an agent with *no* filter (empty set = "all visible") must
/// first seed the allowlist from every registered spec so the hide actually
/// restricts — otherwise removing from an empty set would no-op and leave the
/// tool still callable under the "empty == all visible" contract.
///
/// Note the on-demand tool-pack builder now materialises a concrete visible
/// allowlist at build time, so a freshly built agent is no longer filter-less;
/// the empty-set case is exercised here by explicitly resetting to the "all
/// visible" sentinel, which is the only way a caller reaches it.
#[test]
fn hide_tools_seeds_allowlist_when_no_filter_present() {
    let mut agent = build_minimal_agent_with_definition_name(None);
    assert!(
        !agent.visible_tool_names_for_test().is_empty(),
        "precondition: the tool-pack builder seeds a concrete visible allowlist at build time"
    );
    assert!(
        agent.tool_specs().iter().any(|spec| spec.name == "echo"),
        "precondition: the mock belt includes `echo`"
    );

    // Reset to the "all visible" sentinel so the no-filter seed path below is
    // actually exercised, matching the historical precondition.
    agent.set_visible_tool_names(std::collections::HashSet::new());
    assert!(
        agent.visible_tool_names_for_test().is_empty(),
        "precondition: sentinel reset yields an empty visible-tool set"
    );

    // Hiding a name that isn't on the belt still forces the seed: the set goes
    // from empty ("all visible") to a concrete allowlist of the real tools, so
    // the previously-all-visible belt is now explicitly enumerated.
    agent.hide_tools(&["not_on_belt"]);

    let visible = agent.visible_tool_names_for_test();
    assert!(
        visible.contains("echo"),
        "seeding must materialise the existing belt into a concrete allowlist; visible = {visible:?}"
    );
    assert!(
        !visible.contains("not_on_belt"),
        "an absent hidden name is a harmless no-op; visible = {visible:?}"
    );
}

// ── Issue #4868 — `set_max_tool_iterations` post-construction override ─────

/// `set_max_tool_iterations` directly overrides the runtime cap, independent
/// of whatever the builder resolved it to.
#[test]
fn set_max_tool_iterations_overrides_the_builder_resolved_cap() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let before = agent.agent_config().max_tool_iterations;

    agent.set_max_tool_iterations(200);

    assert_eq!(agent.agent_config().max_tool_iterations, 200);
    assert_ne!(
        200, before,
        "sanity: the override must actually change the cap for this assertion to mean anything"
    );
}

/// Regression for issue #4868's `skill_runtime`/`task_dispatcher` callers:
/// both build the agent via `Agent::from_config_for_agent` (which now stamps
/// the resolved agent definition's own `effective_max_iterations()` — 15 for
/// `orchestrator`), then need a much larger budget (200) for a full
/// workflow/autonomous-task run. `set_max_tool_iterations` must win over
/// whatever the session builder resolved, so the post-construction override
/// actually sticks instead of being silently re-clobbered.
#[test]
fn set_max_tool_iterations_survives_after_definition_backed_construction() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().unwrap();

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let mut config = crate::openhuman::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        action_dir: workspace.path().to_path_buf(),
        ..crate::openhuman::config::Config::default()
    };
    config.http_request.allowed_domains = vec!["*".to_string()];

    let mut agent =
        Agent::from_config_for_agent(&config, "orchestrator").expect("build orchestrator agent");
    assert_eq!(
        agent.agent_config().max_tool_iterations,
        15,
        "precondition: the orchestrator definition's own cap (15) is applied by the builder"
    );

    // Mirrors `skill_runtime::run_machinery`/`task_dispatcher::executor`:
    // apply the much larger workflow/task-run budget AFTER construction.
    const WORKFLOW_RUN_MAX_ITERATIONS: usize = 200;
    agent.set_max_tool_iterations(WORKFLOW_RUN_MAX_ITERATIONS);

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        WORKFLOW_RUN_MAX_ITERATIONS,
        "post-construction override must win over the definition-resolved cap"
    );
}

/// Both resume reads and the turn write are served by the injected locator,
/// with **nothing written under the workspace**. That last assertion is the
/// whole point: it is the proof the `Arc<dyn …>` is a real seam rather than
/// decoration around a hardcoded filesystem call.
#[tokio::test]
async fn fake_locator_substitutes_the_whole_turn_path() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let canned = crate::openhuman::agent::harness::session::transcript::SessionTranscript {
        meta: fake_transcript_meta("thr_fake"),
        messages: vec![
            crate::openhuman::agent::messages::ChatMessage::system("canned system"),
            crate::openhuman::agent::messages::ChatMessage::user("canned question"),
            crate::openhuman::agent::messages::ChatMessage::assistant("canned answer"),
        ],
    };
    let (mut agent, handle) = agent_with_fake_locator(workspace.path(), Some(canned));

    // (1) The stem-keyed resume read.
    agent.try_load_session_transcript();
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("resume prefix came from the fake locator");
    assert_eq!(
        cached
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["canned system", "canned question", "canned answer"]
    );

    // (2) The thread-keyed cold-boot read (cleared first — it no-ops on a warm
    // agent by design).
    agent.cached_transcript_messages = None;
    assert!(agent.seed_resume_from_thread_transcript("thr_fake"));
    assert_eq!(
        agent
            .cached_transcript_messages
            .as_ref()
            .expect("cold-boot prefix")
            .len(),
        3
    );

    // (3) The write.
    let turn = vec![
        crate::openhuman::agent::messages::ChatMessage::user("live question"),
        crate::openhuman::agent::messages::ChatMessage::assistant("live answer"),
    ];
    agent.persist_session_transcript(&turn, 1, 2, 0, 0.0, None);
    let appended = handle.appended.lock();
    assert_eq!(appended.len(), 1, "the turn reached the injected handle");
    assert_eq!(
        appended[0]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["live question", "live answer"]
    );
    assert_eq!(
        agent.session_transcript_path.as_deref(),
        Some(handle.path.as_path()),
        "session_transcript_path is the bound handle's own path — they cannot drift"
    );

    drop(appended);

    // (4) Nothing touched the transcript filesystem. (The #4249 store mirror
    // still runs — it is a separate, gated path this seam does not own — but it
    // never writes `session_raw/`.)
    assert!(
        !workspace.path().join("session_raw").exists(),
        "an injected locator must take the turn path entirely off disk"
    );
}

/// A locator that finds nothing must leave the agent cold, so the caller's
/// prose-seeding fallback still fires.
#[test]
fn fake_locator_with_no_transcript_leaves_the_agent_cold() {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let (mut agent, _handle) = agent_with_fake_locator(workspace.path(), None);

    agent.try_load_session_transcript();
    assert!(agent.cached_transcript_messages.is_none());
    assert!(
        !agent.seed_resume_from_thread_transcript("thr_fake"),
        "an Ok(None) read must report false like a missing file did"
    );
}

/// A resumed transcript must survive into `history`, not just into one request.
///
/// The prefix used to be spliced into the outgoing message list and dropped.
/// Everything downstream reads `history`, so that cost the conversation twice,
/// and both were observed against a live core before this test existed:
///
///  1. The **next** turn went out without it — a ten-message thread answered its
///     following question from four messages. Amnesia first, cache miss second.
///  2. The transcript persisted afterwards is serialized from `history`, so the
///     file written after a resume held only the new turn. `latest_for_agent`
///     hands that file to the next resume, so each restart truncated the thread
///     further.
///
/// Asserting on `history` rather than on a rendered request is deliberate: the
/// request was never the thing that was broken.
#[tokio::test]
async fn a_resumed_transcript_prefix_is_absorbed_into_history() {
    use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};

    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let canned = crate::openhuman::agent::harness::session::transcript::SessionTranscript {
        meta: fake_transcript_meta("thr_resume"),
        messages: vec![
            ChatMessage::system("stored system prompt"),
            ChatMessage::user("first question"),
            ChatMessage::assistant("first answer"),
        ],
    };
    let (mut agent, _handle) = agent_with_fake_locator(workspace.path(), Some(canned));
    agent.try_load_session_transcript();

    // What `turn()` leaves behind on a resumed turn: a freshly rendered system
    // prompt it had to build because `history` was empty, then this turn's user
    // message.
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("freshly rendered prompt")),
        ConversationMessage::Chat(ChatMessage::user("second question")),
    ];

    agent.absorb_resumed_transcript_prefix();

    let rendered: Vec<(&str, &str)> = agent
        .history
        .iter()
        .map(|entry| match entry {
            ConversationMessage::Chat(chat) => (chat.role.as_str(), chat.content.as_str()),
            other => panic!("expected only Chat entries, got {other:?}"),
        })
        .collect();
    assert_eq!(
        rendered,
        vec![
            // The stored prompt wins: it is the prefix the backend has already
            // tokenised, and re-rendering it is what resuming exists to avoid.
            ("system", "stored system prompt"),
            ("user", "first question"),
            ("assistant", "first answer"),
            ("user", "second question"),
        ],
        "the replayed prefix must land in history ahead of this turn, with the \
         freshly rendered system prompt dropped in favour of the stored one"
    );
    assert!(
        agent.cached_transcript_messages.is_none(),
        "the prefix is consumed once — history owns it from here"
    );

    // Defect (1): a second turn appends to a history that still carries the
    // replayed conversation, rather than starting from just its own message.
    agent
        .history
        .push(ConversationMessage::Chat(ChatMessage::user(
            "third question",
        )));
    agent.absorb_resumed_transcript_prefix();
    assert_eq!(
        agent.history.len(),
        5,
        "a later turn must neither lose the replayed prefix nor duplicate it"
    );

    // Defect (2): what persistence serializes is that same full sequence, so
    // the transcript written after a resume is complete and the *next* resume
    // reads a whole thread rather than a truncated one.
    let persisted = agent.tool_dispatcher.to_provider_messages(&agent.history);
    assert_eq!(
        persisted
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "stored system prompt",
            "first question",
            "first answer",
            "second question",
            "third question",
        ],
    );
}
