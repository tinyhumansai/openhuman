use super::*;

#[tokio::test]
async fn turn_with_native_dispatcher_handles_tool_results_variant() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "tc1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                    extra_content: None,
                }],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "done");
    assert!(agent
        .history()
        .iter()
        .any(|msg| matches!(msg, ConversationMessage::ToolResults(_))));
}

#[tokio::test]
async fn turn_with_native_dispatcher_persists_fallback_tool_calls() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(MockProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some(
                    "Checking...\n<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(MockTool)])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        .build()
        .unwrap();

    let response = agent.turn("hi").await.unwrap();
    assert_eq!(response, "done");

    let persisted_calls = agent
        .history()
        .iter()
        .find_map(|msg| match msg {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => Some(tool_calls),
            _ => None,
        })
        .expect("assistant tool calls should be persisted");
    assert_eq!(persisted_calls.len(), 1);
    assert_eq!(persisted_calls[0].name, "echo");
}

/// End-to-end: parent Agent issues a `spawn_subagent` tool call, the
/// runner dispatches a built-in sub-agent (`researcher`) using the
/// same MockProvider, and the parent's next turn folds the sub-agent's
/// text output into the final response.
///
/// This is the highest-level test that exercises:
/// - Agent::turn → execute_tool_call → SpawnSubagentTool::execute
/// - PARENT_CONTEXT task-local visibility
/// - AgentDefinitionRegistry::global lookup
/// - run_subagent → run_inner_loop with the parent's provider
/// - Result returned as a ToolResult and threaded back into history
///
/// Uses the `#[cfg(test)]`-only `__test_inherit_echo` sub-agent
/// (`ModelSpec::Inherit`) rather than `researcher`. After #1710,
/// sub-agents with a `Hint(workload)` spec build a fresh provider via
/// `create_chat_provider(...)` and therefore can't share this test's
/// `MockProvider` — so a Hint sub-agent here would leak the scripted
/// chain. `Inherit` keeps `parent.provider`, which is exactly the
/// plumbing this test asserts. Provider *routing* for Hint sub-agents
/// is covered independently by
/// `subagent_runner::ops::tests::resolve_subagent_source_*`.
// The full spawn_subagent path (parent turn → run_subagent → nested agent
// turn) is a deep async state machine. In debug/coverage builds each future
// frame is large, and the two stacked turns exceed the default ~2 MiB libtest
// per-test thread stack — the thread overflows and SIGABRTs the *entire* test
// process. Because libtest runs tests concurrently, the abort then tags
// whichever unrelated test happened to be in flight as FAILED, producing the
// run-to-run flake reported in issue #5209 (the experience-recall test was the
// most frequent victim). CI only avoided this by exporting a 64 MiB
// `RUST_MIN_STACK`; a raw `cargo test` (e.g. the diff-scoped coverage command)
// has no such env and reliably overflows. Production already drives agent
// turns on an explicit large stack for this exact reason
// (`agent::bus::handle_agent_run_turn_on_large_stack`). Mirror that here so the
// test is self-contained and never aborts the process, regardless of
// `RUST_MIN_STACK`.
#[test]
fn turn_dispatches_spawn_subagent_through_full_path() {
    std::thread::Builder::new()
        .name("spawn-subagent-full-path-test".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(turn_dispatches_spawn_subagent_through_full_path_inner());
        })
        .expect("spawn large-stack test thread")
        .join()
        .expect("large-stack spawn_subagent test thread panicked");
}

/// KV-cache invariant: across multiple turns in the same session, the
/// system-prompt bytes submitted to the provider must be byte-identical,
/// and the model name must not flip. Both are required for the backend's
/// automatic prefix cache to hit — if either changes, the backend must
/// re-prefill the entire prompt every turn.
///
/// This test guards against two regressions:
///   1. A future edit that reintroduces the subsequent-turn system
///      prompt rebuild (see the `learning_enabled` branch we
///      deliberately removed in `turn()`).
///   2. A future edit that reintroduces per-message model
///      classification on the main agent (which would flip the
///      effective model between turns).
#[tokio::test]
async fn system_prompt_and_model_are_byte_stable_across_turns() {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();

    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(vec![
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("first".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("second".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
            crate::openhuman::inference::provider::ChatResponse {
                text: Some("third".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            },
        ]),
        captures: Mutex::new(Vec::new()),
    });

    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut agent = Agent::builder()
        .chat_model(provider.clone() as Arc<dyn ChatModel<()>>)
        .tools(vec![])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(workspace_path)
        // Learning flag is explicitly enabled to prove that the
        // former "rebuild system prompt on subsequent turns" branch
        // is gone — we should still see byte-stable prompts.
        .learning_enabled(true)
        .build()
        .unwrap();

    for prompt in ["first question", "second question", "third question"] {
        agent.turn(prompt).await.unwrap();
    }

    let captures = provider.captures.lock().clone();
    assert_eq!(
        captures.len(),
        3,
        "expected one provider call per turn, got {}",
        captures.len()
    );

    let first_system = captures[0]
        .system_prompt
        .as_ref()
        .expect("first turn should have a system prompt");
    for (idx, cap) in captures.iter().enumerate() {
        let sys = cap
            .system_prompt
            .as_ref()
            .expect("every turn should carry the system prompt");
        assert_eq!(
            sys, first_system,
            "system prompt drifted on turn {} — KV cache prefix broken",
            idx
        );
        assert_eq!(
            cap.model, captures[0].model,
            "model name flipped on turn {} — KV cache namespace broken",
            idx
        );
        assert!(
            !sys.contains("<!-- CACHE_BOUNDARY -->"),
            "system prompt should not leak any cache-boundary marker"
        );
    }
}

/// Regression test for the per-thread transcript resume bug.
///
/// `set_agent_definition_name` is called by the web channel after
/// `Agent::from_config_for_agent("orchestrator")` returns, to scope
/// transcripts per thread (e.g. `"orchestrator_thread-6ad6d"`). Prior
/// to the fix this only updated `agent_definition_name` and left
/// `session_key` pointing at the builder-time name. Persist would
/// then write `session_raw/<ts>_orchestrator.jsonl` while resume
/// searched for `session_raw/<ts>_orchestrator_thread-6ad6d.jsonl`,
/// so every cold-boot turn ran against an empty transcript and the
/// LLM had no conversation history.
///
/// This test pins the contract: after `set_agent_definition_name`,
/// `session_key`'s suffix matches the new (sanitised) name so the
/// next persist+resume pair land on the same file.
#[test]
fn set_agent_definition_name_rewrites_session_key_suffix() {
    let agent_first = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let original_key = agent_first.session_key().to_string();
    assert!(
        original_key.ends_with("_orchestrator"),
        "builder should seed session_key suffix from agent_definition_name; got {original_key}"
    );

    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prefix = agent
        .session_key()
        .split_once('_')
        .map(|(p, _)| p.to_string())
        .expect("session_key must have a `<ts>_<suffix>` shape");

    agent.set_agent_definition_name("orchestrator_thread-6ad6d");

    assert_eq!(agent.agent_definition_name(), "orchestrator_thread-6ad6d");
    assert_eq!(
        agent.session_key(),
        format!("{prefix}_orchestrator_thread-6ad6d"),
        "session_key suffix must track agent_definition_name so transcript persist + \
         resume agree on the file path"
    );
}

/// `set_agent_definition_name` must sanitise non-allowed characters in
/// the new name (matching the builder's policy) so `session_key`
/// never contains anything that would escape the `session_raw/`
/// directory or break filename parsing on disk.
#[test]
fn set_agent_definition_name_sanitises_unsafe_characters() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.set_agent_definition_name("orch/../../etc/passwd thread-6ad6d");
    assert!(
        !agent.session_key().contains('/'),
        "session_key must never contain path separators; got {}",
        agent.session_key()
    );
    assert!(
        !agent.session_key().contains(' '),
        "session_key must never contain whitespace; got {}",
        agent.session_key()
    );
}

/// Cold-boot resume from the conversation JSONL works even when no
/// matching transcript file exists. The web channel calls
/// `seed_resume_from_messages` on the cache-miss path so the agent
/// sees prior conversation context immediately, instead of having to
/// wait for a transcript to be persisted under the new
/// thread-scoped name.
#[test]
fn seed_resume_from_messages_primes_cached_transcript() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prior = vec![
        ("user".to_string(), "what is btc price".to_string()),
        ("agent".to_string(), "$80,000".to_string()),
        // Trailing user message that the caller is about to pass to
        // run_single — must be deduped from the cached prefix.
        ("user".to_string(), "what did i just ask".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "what did i just ask")
        .expect("seed");

    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // [system, user(btc), agent(80k)] — trailing user was deduped.
    assert_eq!(cached.len(), 3);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].role, "user");
    assert_eq!(cached[1].content, "what is btc price");
    assert_eq!(cached[2].role, "assistant");
    assert_eq!(cached[2].content, "$80,000");
}

/// `seed_resume_from_messages` must not stomp the existing context if
/// the agent has already been warmed (in-process session cache hit).
/// Otherwise the cache-miss branch in the web channel would erase
/// real progress whenever the caller defensively invoked seeding.
#[test]
fn seed_resume_from_messages_is_noop_on_warm_agent() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.cached_transcript_messages = Some(vec![
        crate::openhuman::agent::messages::ChatMessage::system("warm prefix"),
        crate::openhuman::agent::messages::ChatMessage::user("hi"),
    ]);
    agent
        .seed_resume_from_messages(vec![("user".into(), "different".into())], "different")
        .expect("seed");
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("still populated");
    assert_eq!(cached.len(), 2);
    assert_eq!(cached[0].content, "warm prefix");
}

/// Trailing user message that does NOT match the current incoming
/// message must be preserved — the dedup heuristic only fires on
/// exact match because the conversation JSONL is the source of truth
/// and may legitimately contain back-to-back user messages (e.g. the
/// thread-7242c case where an interrupted turn left the prior user
/// message un-replied).
#[test]
fn seed_resume_from_messages_preserves_unmatched_trailing_user() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    let prior = vec![
        ("user".to_string(), "earlier question".to_string()),
        ("agent".to_string(), "earlier answer".to_string()),
        ("user".to_string(), "stranded follow-up".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "completely different new turn")
        .expect("seed");
    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // [system, user, agent, user] — trailing kept because it doesn't
    // match the current turn's user input.
    assert_eq!(cached.len(), 4);
    assert_eq!(cached[3].role, "user");
    assert_eq!(cached[3].content, "stranded follow-up");
}

#[test]
fn seed_resume_from_messages_respects_history_window_bound() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 4;
    let prior = vec![
        ("user".to_string(), "u1".to_string()),
        ("agent".to_string(), "a1".to_string()),
        ("user".to_string(), "u2".to_string()),
        ("agent".to_string(), "a2".to_string()),
        ("user".to_string(), "u3".to_string()),
        ("agent".to_string(), "a3".to_string()),
    ];
    agent
        .seed_resume_from_messages(prior, "new turn")
        .expect("seed");

    let cached = agent
        .cached_transcript_messages
        .as_ref()
        .expect("cache populated");
    // max_history_messages=4 keeps [system + last 3 messages].
    assert_eq!(cached.len(), 4);
    assert_eq!(cached[0].role, "system");
    assert_eq!(cached[1].content, "a2");
    assert_eq!(cached[2].content, "u3");
    assert_eq!(cached[3].content, "a3");
}

#[test]
fn bound_cached_transcript_messages_without_system_prefix_keeps_tail() {
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 3;

    let messages = vec![
        crate::openhuman::agent::messages::ChatMessage::user("u1"),
        crate::openhuman::agent::messages::ChatMessage::assistant("a1"),
        crate::openhuman::agent::messages::ChatMessage::user("u2"),
        crate::openhuman::agent::messages::ChatMessage::assistant("a2"),
        crate::openhuman::agent::messages::ChatMessage::user("u3"),
    ];
    let bounded = agent.bound_cached_transcript_messages(messages);
    assert_eq!(bounded.len(), 3);
    assert_eq!(bounded[0].content, "u2");
    assert_eq!(bounded[1].content, "a2");
    assert_eq!(bounded[2].content, "u3");
}

/// The cached-transcript resume path operates on wire-form `ChatMessage`s. When
/// the window cut lands so the tail opens on a `tool` result whose `tool_calls`
/// opener fell outside the window, `bound_cached_transcript_messages` must snap
/// past it — a leading `tool` message has no preceding `tool_calls` and the
/// provider 400s (surfacing as "Something went wrong").
#[test]
fn bound_cached_transcript_messages_snaps_past_leading_orphan_tool() {
    use crate::openhuman::agent::messages::ChatMessage;

    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));
    agent.config.max_history_messages = 3;

    // 5 messages, cap 3: the tail slice is [tool(a), user(u2), assistant(a2)];
    // the assistant `tool_calls` opener fell outside the window.
    let messages = vec![
        ChatMessage::assistant(
            r#"{"content":"calling","tool_calls":[{"id":"call_a","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"orphaned"}"#),
        ChatMessage::user("u2"),
        ChatMessage::assistant("a2"),
        ChatMessage::user("u3"),
    ];

    let bounded = agent.bound_cached_transcript_messages(messages);

    assert!(
        bounded.first().map(|m| m.role.as_str()) != Some("tool"),
        "window must not open on an orphaned tool result"
    );
    assert!(
        !bounded.iter().any(|m| m.role == "tool"),
        "the orphaned tool result must be dropped"
    );
    // tail [tool, u2, a2, u3] -> drop leading tool -> [u2, a2, u3].
    assert_eq!(
        bounded
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["u2", "a2", "u3"]
    );
}

/// A durable tool with a caller-chosen name, for the tests that pin how the
/// durable registry and the synthesised delegation set relate.
struct NamedTool(&'static str);

#[async_trait]
impl Tool for NamedTool {
    /// The caller-chosen name.
    fn name(&self) -> &str {
        self.0
    }

    /// Fixed marker text, so a test can tell it from a synthesised delegate.
    fn description(&self) -> &str {
        "durable"
    }

    /// A schema no synthesised delegate produces, so a spec can be attributed.
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"durable": {"type": "boolean"}}})
    }

    /// Returns a fixed success; dispatch itself is not under test here.
    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> Result<crate::openhuman::tools::ToolResult> {
        Ok(crate::openhuman::tools::ToolResult::success("durable"))
    }
}

/// A connected Composio toolkit with the given slug, for driving refreshes.
fn connected(slug: &str) -> crate::openhuman::agent::context::prompt::ConnectedIntegration {
    crate::openhuman::agent::context::prompt::ConnectedIntegration {
        toolkit: slug.into(),
        description: slug.into(),
        tools: vec![],
        gated_tools: vec![],
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    }
}

/// Regression for #6145, revoke direction: a delegate whose toolkit was
/// disconnected must leave the executable set and the policy snapshot in the
/// same pass its spec is withdrawn, even while an in-flight turn holds clones.
///
/// This was the damaging half of the drift: the spec went, but the instance
/// stayed in the shared `tools` Arc, the policy session built from that Arc
/// kept allowing it, and the adapter registered from it kept advertising it —
/// so a disconnected integration's delegate remained callable.
#[test]
fn revoked_delegate_leaves_instances_and_policy_with_its_spec_while_tool_arc_is_shared() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;
    const DELEGATE: &str = "delegate_to_integrations_agent";

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_definition_name(Some("orchestrator"));

    agent.set_connected_integrations(vec![connected("gmail"), connected("notion")]);
    agent.refresh_delegation_tools();
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string(), "notion".to_string()]
    );

    // An in-flight turn holds both sets for its whole duration.
    let _durable_in_flight = agent.tools_arc();
    let synthesized_in_flight = agent.synthesized_tools_arc();

    // notion is disconnected: the enum shrinks and the instance follows it.
    agent.set_connected_integrations(vec![connected("gmail")]);
    agent.refresh_delegation_tools();
    assert_eq!(
        integration_delegate_toolkit_enum(&agent),
        vec!["gmail".to_string()]
    );
    super::assert_synthesized_delegates_are_executable(&agent);

    // Everything is disconnected: the delegate leaves every surface at once.
    agent.set_connected_integrations(Vec::new());
    agent.refresh_delegation_tools();
    assert!(
        !agent.tool_specs().iter().any(|spec| spec.name == DELEGATE),
        "the withdrawn delegate must not keep a spec"
    );
    assert!(
        !agent
            .synthesized_tools_arc()
            .iter()
            .any(|tool| tool.name() == DELEGATE),
        "the executable instance must go with the spec — this is what stayed \
         behind before #6145"
    );
    assert!(!agent
        .all_tool_refs()
        .iter()
        .any(|tool| tool.name() == DELEGATE));
    assert!(
        !agent.tool_policy_session.is_allowed(DELEGATE)
            && !agent.tool_policy_session.decisions.contains_key(DELEGATE),
        "a withdrawn delegate must not keep a policy allowance"
    );
    super::assert_synthesized_delegates_are_executable(&agent);

    // The turn that started before the revoke still sees its own coherent set.
    assert!(
        synthesized_in_flight
            .iter()
            .any(|tool| tool.name() == DELEGATE),
        "an in-flight turn keeps the snapshot it started with"
    );
}

/// A durable tool that owns a delegate's name wins on every surface, at build
/// time and on every refresh — and a refresh must never withdraw the durable
/// tool's spec when it re-synthesises the colliding delegate.
///
/// The old refresh put the unfiltered synthesised names into its mask, so the
/// next refresh's spec `retain` dropped the durable tool's spec and appended
/// the delegate's: the model then read the delegate's schema for a name whose
/// first-wins dispatch ran the durable tool.
#[test]
fn a_durable_tool_owning_a_delegate_name_wins_everywhere_across_refreshes() {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;
    const DELEGATE: &str = "delegate_to_integrations_agent";

    AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent = build_minimal_agent_with_tool_sets(
        vec![Box::new(MockTool), Box::new(NamedTool(DELEGATE))],
        Vec::new(),
        Some("orchestrator"),
    );
    let durable_schema = NamedTool(DELEGATE).parameters_schema();

    for toolkits in [vec!["gmail"], vec!["gmail", "notion"], vec!["notion"]] {
        agent.set_connected_integrations(toolkits.iter().map(|slug| connected(slug)).collect());
        agent.refresh_delegation_tools();

        assert!(
            !agent
                .synthesized_tools_arc()
                .iter()
                .any(|tool| tool.name() == DELEGATE),
            "the colliding delegate must not be synthesised beside the durable tool"
        );
        let specs: Vec<&std::sync::Arc<crate::openhuman::tools::ToolSpec>> = agent
            .tool_specs()
            .iter()
            .filter(|spec| spec.name == DELEGATE)
            .collect();
        assert_eq!(specs.len(), 1, "exactly one spec may carry the name");
        assert_eq!(
            specs[0].parameters, durable_schema,
            "and it must be the durable tool's schema — a refresh must not swap it \
             for the delegate's"
        );
        assert_eq!(
            agent
                .all_tool_refs()
                .iter()
                .filter(|tool| tool.name() == DELEGATE)
                .count(),
            1
        );
        assert!(agent.tool_policy_session.is_allowed(DELEGATE));
        super::assert_synthesized_delegates_are_executable(&agent);
    }
}

/// The builder keeps the synthesised set beside the durable registry, never
/// inside it, and drops a synthesised name a durable tool already owns.
#[test]
fn builder_holds_synthesized_tools_apart_from_the_durable_registry() {
    let agent = build_minimal_agent_with_tool_sets(
        vec![Box::new(MockTool)],
        vec![
            Box::new(NamedTool("delegate_probe")),
            Box::new(NamedTool("echo")),
        ],
        None,
    );

    assert!(
        agent.tools().iter().all(|tool| tool.name() == "echo"),
        "the durable registry must hold only what was handed to `tools`"
    );
    let synthesized: Vec<String> = agent
        .synthesized_tools_arc()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(
        synthesized,
        vec!["delegate_probe".to_string()],
        "the synthesised `echo` collides with the durable tool and must be dropped"
    );
    assert_eq!(
        agent
            .tool_specs()
            .iter()
            .filter(|spec| spec.name == "echo")
            .count(),
        1,
        "the durable tool keeps the only `echo` spec"
    );
    assert!(agent
        .tool_specs()
        .iter()
        .any(|spec| spec.name == "delegate_probe"));
    assert!(agent.tool_policy_session.is_allowed("delegate_probe"));
    let expected_mask: std::collections::HashSet<String> =
        std::iter::once("delegate_probe".to_string()).collect();
    assert_eq!(agent.synthesized_tool_names, expected_mask);
    super::assert_synthesized_delegates_are_executable(&agent);
}
