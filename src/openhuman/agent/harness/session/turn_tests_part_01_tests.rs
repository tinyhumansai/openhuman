use super::*;

#[test]
fn trim_history_preserves_system_and_keeps_latest_non_system_entries() {
    let mut agent = make_agent(None);
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        ConversationMessage::Chat(ChatMessage::user("u1")),
        ConversationMessage::Chat(ChatMessage::assistant("a1")),
        ConversationMessage::Chat(ChatMessage::user("u2")),
        ConversationMessage::Chat(ChatMessage::assistant("a2")),
    ];

    agent.trim_history();

    assert_eq!(agent.history.len(), 4);
    assert!(matches!(&agent.history[0], ConversationMessage::Chat(msg) if msg.role == "system"));
    assert!(agent
        .history
        .iter()
        .all(|msg| !matches!(msg, ConversationMessage::Chat(chat) if chat.content == "u1")));
    assert!(agent
        .history
        .iter()
        .any(|msg| matches!(msg, ConversationMessage::Chat(chat) if chat.content == "a2")));
}

/// When the `max_history_messages` cap drops an `AssistantToolCalls` opener but
/// keeps its `ToolResults`, the window would otherwise open on an orphaned tool
/// result — serialized, a `tool` message with no preceding `tool_calls`, which
/// the provider rejects (the 400 that surfaces as "Something went wrong").
/// `trim_history` must snap past the orphan so the window starts on a clean turn.
#[test]
fn trim_history_snaps_past_orphaned_tool_results() {
    use crate::openhuman::agent::messages::ToolResultMessage;
    use crate::openhuman::inference::provider::ToolCall;

    let mut agent = make_agent(None); // max_history_messages = 3
    agent.history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        // This opener is the oldest non-system entry, so the cap drops it...
        ConversationMessage::AssistantToolCalls {
            text: Some("calling".into()),
            tool_calls: vec![ToolCall {
                id: "call_x".into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            }],
            reasoning_content: None,
            extra_metadata: None,
        },
        // ...orphaning this result at the head of the kept window.
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "call_x".into(),
            content: "result".into(),
        }]),
        ConversationMessage::Chat(ChatMessage::user("u2")),
        ConversationMessage::Chat(ChatMessage::assistant("a2")),
    ];

    agent.trim_history();

    assert!(
        !agent
            .history
            .iter()
            .any(|m| matches!(m, ConversationMessage::ToolResults(_))),
        "orphaned ToolResults must be dropped, not left at the window head"
    );
    assert!(
        matches!(agent.history.first(), Some(ConversationMessage::Chat(c)) if c.role == "system"),
        "system message is preserved"
    );
    // system + u2 + a2 (the bisected cycle is gone entirely).
    assert_eq!(agent.history.len(), 3);
}

#[tokio::test]
async fn build_parent_context_and_sanitize_helpers_cover_snapshot_paths() {
    let mut agent = make_agent(None);
    agent.last_memory_context = Some("remember this".into());
    agent.workflows = vec![crate::openhuman::skills::Workflow {
        name: "demo".into(),
        ..Default::default()
    }];

    let parent = agent.build_parent_execution_context();
    assert_eq!(parent.model_name, agent.model_name);
    assert_eq!(parent.temperature, agent.temperature);
    assert_eq!(parent.memory_context.as_deref(), Some("remember this"));
    assert_eq!(parent.session_id, "turn-test-session");
    assert_eq!(parent.channel, "turn-test-channel");
    assert_eq!(parent.workflows.len(), 1);

    assert_eq!(sanitize_learned_entry("   "), "");
    assert_eq!(
        sanitize_learned_entry("Bearer abcdef"),
        "[redacted: potential secret]"
    );
    let long = "x".repeat(500);
    assert_eq!(sanitize_learned_entry(&long).chars().count(), 200);
    // A profile subtree that was never written. Named rather than `"memory"`
    // because the shared subtree is the driver's now (#5560), and what this
    // line is here to cover is the host-local scan's empty answer.
    assert!(
        collect_tree_root_summaries(agent.workspace_dir(), "memory-absent", 8_000, 32_000)
            .await
            .is_empty()
    );
}

#[test]
fn build_parent_context_propagates_own_descriptor_on_root_turn() {
    // Regression (PR #5118 review, Codex): on a ROOT chat turn `current_parent()`
    // is `None`, so the parent snapshot must fall back to the agent's OWN
    // descriptor. Without it, a dedicated-workspace profile's descriptor never
    // reaches subagents spawned via spawn_subagent/spawn_async_subagent, and they
    // silently fall back to the shared action_dir instead of
    // `<action_dir>/profiles/<id>`.
    let descriptor = tinyagents_harness::workspace::WorkspaceDescriptor::new(
        std::path::PathBuf::from("/tmp/act/profiles/alice"),
    )
    .with_policy_id("openhuman.profile:alice");

    let mut agent = make_agent(None);
    // No ambient parent context is installed in this test, so current_parent()
    // is None — exactly the root-turn scenario.
    agent.workspace_descriptor = Some(descriptor);

    let parent = agent.build_parent_execution_context();
    assert_eq!(
        parent.workspace_descriptor.as_ref().map(|d| d.root.clone()),
        Some(std::path::PathBuf::from("/tmp/act/profiles/alice")),
        "root turn must propagate the agent's own profile descriptor to spawned subagents"
    );
    assert_eq!(
        parent
            .workspace_descriptor
            .as_ref()
            .map(|d| d.policy_id.clone()),
        Some("openhuman.profile:alice".to_string()),
    );
}

#[test]
fn build_parent_context_has_no_descriptor_without_profile_or_parent() {
    // A profile-less root turn (no ambient parent, no own descriptor) keeps the
    // snapshot's descriptor `None` so shared-action_dir behaviour is unchanged.
    let agent = make_agent(None);
    let parent = agent.build_parent_execution_context();
    assert!(parent.workspace_descriptor.is_none());
}

#[tokio::test]
async fn collect_tree_root_summaries_maps_namespace_body_and_timestamp() {
    // #2944: the wrapper must carry the root node's `updated_at` from the
    // store tuple into the `NamespaceSummary` the prompt renderer stamps.
    //
    // Asserted over a **profile** subtree since #5560: the mapping is the same
    // one both arms share, and the profile arm is the one that still scans a
    // caller-named workspace. The shared `"memory"` arm now answers from the
    // bound driver, which has no way to be pointed at this temp directory.
    use crate::openhuman::config::Config;
    use tinycortex::memory::tree::runtime::{
        derive_parent_id, estimate_tokens, level_from_node_id, TreeNode,
    };
    use tinymemory_core::tree::tree_runtime::store::write_node;

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = Config {
        workspace_dir: workspace.clone(),
        ..Config::default()
    };

    let updated_at = chrono::DateTime::parse_from_rfc3339("2026-05-25T09:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let summary = "Distilled activities summary.";
    let node = TreeNode {
        node_id: "root".to_string(),
        namespace: "activities".to_string(),
        level: level_from_node_id("root"),
        parent_id: derive_parent_id("root"),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count: 0,
        created_at: updated_at,
        updated_at,
        metadata: None,
    };
    write_node(&config, &node).unwrap();
    // `write_node` only knows `<workspace>/memory`; rename it into the profile
    // layout the host-local arm reads.
    std::fs::rename(workspace.join("memory"), workspace.join("memory-alice")).unwrap();

    let summaries = collect_tree_root_summaries(&workspace, "memory-alice", 8_000, 32_000).await;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].namespace, "activities");
    assert_eq!(summaries[0].body, summary);
    assert_eq!(summaries[0].updated_at, updated_at);
}

#[tokio::test]
async fn collect_tree_root_summaries_reads_only_profile_memory_subtree() {
    use crate::openhuman::config::Config;
    use tinycortex::memory::tree::runtime::{
        derive_parent_id, estimate_tokens, level_from_node_id, TreeNode,
    };
    use tinymemory_core::tree::tree_runtime::store::write_node;

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = Config {
        workspace_dir: workspace.clone(),
        ..Config::default()
    };
    let now = chrono::Utc::now();
    let node = TreeNode {
        node_id: "root".into(),
        namespace: "private".into(),
        level: level_from_node_id("root"),
        parent_id: derive_parent_id("root"),
        summary: "Alice-only context".into(),
        token_count: estimate_tokens("Alice-only context"),
        child_count: 0,
        created_at: now,
        updated_at: now,
        metadata: None,
    };
    write_node(&config, &node).unwrap();
    std::fs::rename(workspace.join("memory"), workspace.join("memory-alice")).unwrap();

    // A *different* profile's subtree, not `"memory"`: since #5560 the shared
    // arm answers from the bound driver rather than from this temp workspace,
    // so asking it here would be asserting about a store this test never
    // wrote. Bob is the isolation the assertion is actually about.
    assert!(
        collect_tree_root_summaries(&workspace, "memory-bob", 8_000, 32_000)
            .await
            .is_empty()
    );
    let summaries = collect_tree_root_summaries(&workspace, "memory-alice", 8_000, 32_000).await;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].body, "Alice-only context");
}

#[tokio::test]
async fn transcript_roundtrip_work() {
    let mut agent = make_agent(None);

    let messages = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("done"),
    ];
    agent.persist_session_transcript(&messages, 10, 5, 3, 0.25, None);
    assert!(agent.session_transcript_path.is_some());

    let loaded = transcript::read_transcript(agent.session_transcript_path.as_ref().unwrap())
        .expect("transcript should be readable");
    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.meta.input_tokens, 10);

    let mut resumed = make_agent(None);
    resumed.workspace_dir = agent.workspace_dir.clone();
    resumed.agent_definition_name = agent.agent_definition_name.clone();
    resumed.try_load_session_transcript();
    assert_eq!(
        resumed.cached_transcript_messages.as_ref().map(|m| m.len()),
        Some(3)
    );
}

#[tokio::test]
async fn transcript_resume_is_bounded_by_max_history_messages() {
    let mut writer = make_agent(None);
    let mut messages = vec![ChatMessage::system("sys")];
    for idx in 0..8 {
        messages.push(ChatMessage::user(format!("u{idx}")));
        messages.push(ChatMessage::assistant(format!("a{idx}")));
    }
    writer.persist_session_transcript(&messages, 0, 0, 0, 0.0, None);

    let mut resumed = make_agent(None);
    resumed.workspace_dir = writer.workspace_dir.clone();
    resumed.agent_definition_name = writer.agent_definition_name.clone();
    resumed.config.max_history_messages = 5;
    resumed.try_load_session_transcript();

    let cached = resumed
        .cached_transcript_messages
        .as_ref()
        .expect("resume cache should be populated");
    assert_eq!(cached.len(), 5);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].content, "u6");
    assert_eq!(cached[2].content, "a6");
    assert_eq!(cached[3].content, "u7");
    assert_eq!(cached[4].content, "a7");
}

#[tokio::test]
async fn transcript_resume_uses_profile_scoped_raw_directory() {
    let mut shared = make_agent(None);
    shared.persist_session_transcript(
        &[
            ChatMessage::system("shared-system"),
            ChatMessage::user("shared-user"),
        ],
        0,
        0,
        0,
        0.0,
        None,
    );

    let mut profile = make_agent(None);
    profile.workspace_dir = shared.workspace_dir.clone();
    profile.agent_definition_name = shared.agent_definition_name.clone();
    profile.session_raw_subdir = "session_raw-alice".to_string();
    profile.persist_session_transcript(
        &[
            ChatMessage::system("profile-system"),
            ChatMessage::user("profile-user"),
        ],
        0,
        0,
        0,
        0.0,
        None,
    );

    let mut resumed = make_agent(None);
    resumed.workspace_dir = shared.workspace_dir.clone();
    resumed.agent_definition_name = shared.agent_definition_name.clone();
    resumed.session_raw_subdir = "session_raw-alice".to_string();
    resumed.try_load_session_transcript();

    let cached = resumed
        .cached_transcript_messages
        .expect("profile transcript");
    assert!(cached
        .iter()
        .any(|message| message.content == "profile-user"));
    assert!(cached
        .iter()
        .all(|message| message.content != "shared-user"));
}

// NOTE: The `execute_tool_call_*` tests that exercised the legacy per-call
// direct tool executor (`Agent::execute_tool_call`) were removed during the
// tinyagents migration. The direct executor and its test-only parity shim
// (`session/agent_tool_exec.rs`) were deleted (commit 8aba23886); tool
// execution now happens inside the tinyagents graph turn, so these tests target
// an API that no longer exists. Removed: blocks_invisible_tool_and_emits_events,
// reports_unknown_tool, rewrites_legacy_run_skill_for_builtin_cron_tools,
// rewrites_run_workflow_for_builtin_cron_tools,
// denies_tool_above_channel_permission (and, below,
// denies_by_policy_before_tool_runs, threads_generated_tool_context_into_policy,
// applies_inline_result_budget).

#[test]
fn system_prompt_includes_tool_policy_boundary() {
    let provider: Arc<dyn ChatModel<()>> = Arc::new(DummyProvider);
    let mut config = crate::openhuman::config::AgentConfig::default();
    config
        .channel_permissions
        .insert("turn-test-channel".into(), "read_only".into());
    let agent = make_agent_with_builder(
        provider,
        vec![
            Box::new(EchoTool),
            Box::new(CountingWriteTool {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        ],
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("prompt");

    assert!(prompt.contains("## Tool Policy Boundary"));
    assert!(prompt.contains("Allowed tools: echo"));
    assert!(prompt.contains("Restricted tools: 1 omitted by policy"));
    assert!(!prompt.contains("write_notes"));
}

#[test]
fn set_agent_definition_name_refreshes_tool_policy_identity() {
    let provider: Arc<dyn ChatModel<()>> = Arc::new(DummyProvider);
    let mut config = crate::openhuman::config::AgentConfig::default();
    config
        .channel_permissions
        .insert("turn-test-channel".into(), "read_only".into());
    let mut agent = make_agent_with_builder(
        provider,
        vec![
            Box::new(EchoTool),
            Box::new(CountingWriteTool {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        ],
        vec![],
        config,
        crate::openhuman::config::ContextConfig::default(),
    );

    agent.set_agent_definition_name("renamed_agent");

    assert_eq!(agent.tool_policy_session.profile.agent_id, "renamed_agent");
    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("prompt");
    assert!(prompt.contains("Agent: renamed_agent"));
}

// Removed: execute_tool_call_denies_by_policy_before_tool_runs and
// execute_tool_call_threads_generated_tool_context_into_policy — see the note
// above; they exercised the deleted direct tool executor.

#[tokio::test]
async fn turn_runs_full_tool_cycle_with_context_and_hooks() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some(
                    "preface <tool_call>{\"name\":\"echo\",\"arguments\":{\"value\":1}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("final answer".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let hook_calls = Arc::new(AsyncMutex::new(Vec::<TurnContext>::new()));
    let hook_notify = Arc::new(Notify::new());
    let hooks: Vec<Arc<dyn PostTurnHook>> = vec![Arc::new(RecordingHook {
        calls: Arc::clone(&hook_calls),
        notify: Arc::clone(&hook_notify),
    })];

    let mut agent = make_agent_with_builder(
        provider,
        vec![Box::new(EchoTool)],
        hooks,
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    let response = agent
        .turn("hello world")
        .await
        .expect("turn should succeed");
    assert_eq!(response, "final answer");
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::AssistantToolCalls {
            text, tool_calls, ..
        }
            if text.as_deref().is_some_and(|value| value.contains("preface")) && tool_calls.len() == 1
    )));
    assert!(agent.history.iter().any(|message| matches!(
        message,
        ConversationMessage::Chat(chat) if chat.role == "assistant" && chat.content == "final answer"
    )));

    timeout(Duration::from_secs(1), async {
        loop {
            if !hook_calls.lock().await.is_empty() {
                break;
            }
            hook_notify.notified().await;
        }
    })
    .await
    .expect("hook should fire");

    let recorded_hooks = hook_calls.lock().await;
    assert_eq!(recorded_hooks.len(), 1);
    assert_eq!(recorded_hooks[0].assistant_response, "final answer");
    assert_eq!(recorded_hooks[0].iteration_count, 2);
    assert_eq!(recorded_hooks[0].tool_calls.len(), 1);
    assert_eq!(recorded_hooks[0].tool_calls[0].name, "echo");
    drop(recorded_hooks);

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0][0].role, "system");
    assert!(requests[0][1].content.contains("hello world"));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "assistant" && msg.content.contains("preface")));
    assert!(requests[1]
        .iter()
        .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]")));
}

#[tokio::test]
async fn turn_triggers_configured_memory_agent_before_parent_prompt() {
    crate::openhuman::memory::host_impls::install_for_tests();
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    crate::openhuman::memory::host_impls::install_for_tests();
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("built-in agent definitions should load");
    assert!(
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .and_then(|registry| registry.get("agent_memory"))
            .is_some()
    );

    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![
            Ok(ChatResponse {
                text: Some("memory context: user prefers concise Rust changes".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
            Ok(ChatResponse {
                text: Some("parent final".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }),
        ]),
        requests: AsyncMutex::new(Vec::new()),
        tool_counts: AsyncMutex::new(Vec::new()),
    });
    let provider: Arc<dyn ChatModel<()>> = provider_impl.clone();
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    // The triggered memory agent runs through `run_subagent`, whose
    // deterministic fast path loads the host config and queries whatever memory
    // tree it points at. Keep that inside this test's scratch workspace so the
    // fast path finds nothing and the model-driven walk (the two-call sequence
    // asserted below) is what actually runs.
    let _workspace_env = WorkspaceEnvGuard::set(&workspace_path);
    let memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    // The embedding seam, as above.
    crate::openhuman::memory::host_impls::install_for_tests();
    let mem: Arc<dyn Memory> =
        Arc::from(tinymemory_core::store::create_memory(&memory_cfg, &workspace_path).unwrap());

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
    assert_eq!(
        agent.trigger_memory_agent,
        crate::openhuman::agent::harness::definition::TriggerMemoryAgent::Always
    );

    let response = agent
        .turn("Implement the memory trigger.")
        .await
        .expect("turn should succeed");
    assert_eq!(response, "parent final");

    let requests = provider_impl.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].iter().any(|msg| {
        msg.role == "user" && msg.content.contains("Implement the memory trigger.")
    }));
    assert!(requests[1].iter().any(|msg| {
        msg.role == "user"
            && msg.content.contains("## Memory agent context")
            && msg
                .content
                .contains("memory context: user prefers concise Rust changes")
            && msg.content.contains("Implement the memory trigger.")
    }));
}

/// #1725: a per-turn `suppress_tools` override sends an EMPTY tool schema to the
/// provider, so a chat / small-talk turn cannot enter the tool loop — even
/// though the agent was built WITH a tool. The default (no override) path still
/// offers the tool, proving the suppression is per-turn and not a rebuild.
#[tokio::test]
async fn turn_override_suppress_tools_sends_empty_tool_schema() {
    let provider_impl = Arc::new(SequenceProvider {
        responses: AsyncMutex::new(vec![Ok(ChatResponse {
            text: Some("hi there".into()),
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
        Vec::new(),
        crate::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            max_history_messages: 10,
            ..crate::openhuman::config::AgentConfig::default()
        },
        crate::openhuman::config::ContextConfig::default(),
    );

    agent.set_next_turn_overrides(crate::openhuman::agent::harness::session::TurnOverrides {
        suppress_tools: true,
        ..Default::default()
    });
    let response = agent.turn("hey").await.expect("turn should succeed");
    assert_eq!(response, "hi there");

    let counts = provider_impl.tool_counts.lock().await;
    assert_eq!(counts.len(), 1, "small-talk turn is a single provider call");
    assert_eq!(
        counts[0], 0,
        "suppress_tools must send an empty tool schema (no tool loop possible)"
    );
}
