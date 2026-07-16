use super::*;

fn make_def(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.into(),
        when_to_use: "test".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("system".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

#[test]
fn registry_insert_and_lookup() {
    let mut reg = AgentDefinitionRegistry::default();
    reg.insert(make_def("alpha"));
    reg.insert(make_def("beta"));
    assert_eq!(reg.len(), 2);
    assert!(reg.get("alpha").is_some());
    assert!(reg.get("beta").is_some());
    assert!(reg.get("missing").is_none());
}

#[test]
fn registry_replace_preserves_order() {
    let mut reg = AgentDefinitionRegistry::default();
    reg.insert(make_def("alpha"));
    reg.insert(make_def("beta"));
    let mut updated = make_def("alpha");
    updated.when_to_use = "replaced".into();
    reg.insert(updated);

    let list: Vec<&str> = reg.list().iter().map(|d| d.id.as_str()).collect();
    assert_eq!(list, vec!["alpha", "beta"]);
    assert_eq!(reg.get("alpha").unwrap().when_to_use, "replaced");
}

#[test]
fn model_spec_resolve_inherit_uses_parent() {
    let spec = ModelSpec::Inherit;
    assert_eq!(spec.resolve("parent-model"), "parent-model");
}

#[test]
fn model_spec_resolve_exact_uses_name() {
    let spec = ModelSpec::Exact("kimi-k2".into());
    assert_eq!(spec.resolve("parent-model"), "kimi-k2");
}

#[test]
fn model_spec_resolve_hint_appends_v1() {
    let spec = ModelSpec::Hint("coding".into());
    assert_eq!(spec.resolve("parent-model"), "coding-v1");
}

#[test]
fn model_spec_resolve_vision_hint_yields_vision_v1() {
    // The vision sub-agent's `hint = "vision"` must resolve to the `vision-v1`
    // tier alias — which `oh_tier_supports_vision` reports as image-capable.
    let spec = ModelSpec::Hint("vision".into());
    assert_eq!(spec.resolve("parent-model"), "vision-v1");
}

#[test]
fn display_name_falls_back_to_id() {
    let def = make_def("alpha");
    assert_eq!(def.display_name(), "alpha");
    let mut def2 = make_def("beta");
    def2.display_name = Some("Beta Specialist".into());
    assert_eq!(def2.display_name(), "Beta Specialist");
}

// ── subagents parsing ─────────────────────────────────────────────

/// Parses a minimal TOML document with a `subagents` list containing
/// both a bare agent-id string and an inline `{ skills = "*" }` table.
/// Ensures the `#[serde(untagged)]` enum routes each shape to the
/// correct variant without the TOML needing explicit tags.
///
/// NOTE: `subagents = [...]` must appear **before** the `[tools]`
/// table header in the TOML — once you open a TOML table section,
/// every subsequent top-level key is consumed by that table, so
/// `subagents` placed after `[tools]` would be parsed as
/// `tools.subagents` and fail because `ToolScope` is an enum, not
/// a struct with a `subagents` field.
#[test]
fn subagents_parses_mixed_string_and_table_entries() {
    let toml_src = r#"
id = "orchestrator"
when_to_use = "Routes work to the right specialist"
temperature = 0.4
max_iterations = 15

subagents = [
"researcher",
"code_executor",
{ skills = "*" },
]

[tools]
named = ["query_memory"]
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert_eq!(def.subagents.len(), 3);
    assert_eq!(
        def.subagents[0],
        SubagentEntry::AgentId("researcher".into())
    );
    assert_eq!(
        def.subagents[1],
        SubagentEntry::AgentId("code_executor".into())
    );
    assert_eq!(
        def.subagents[2],
        SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })
    );
}

#[test]
fn subagents_section_parses_allowlist_entries() {
    let toml_src = r#"
id = "orchestrator"
when_to_use = "Routes work to the right specialist"
temperature = 0.4
max_iterations = 15

[subagents]
allowlist = [
    "researcher",
    "code_executor",
    { skills = "*" },
]

[tools]
named = ["query_memory"]
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert_eq!(
        def.subagents,
        vec![
            SubagentEntry::AgentId("researcher".into()),
            SubagentEntry::AgentId("code_executor".into()),
            SubagentEntry::Skills(SkillsWildcard { skills: "*".into() }),
        ]
    );
}

/// `subagents` is optional — omitting it should yield an empty Vec
/// rather than a deserialization error. Most non-delegating agents
/// (archivist, code_executor, etc.) will not list any.
#[test]
fn subagents_defaults_to_empty_when_omitted() {
    let toml_src = r#"
id = "code_executor"
when_to_use = "Runs code and shell commands"
temperature = 0.7
max_iterations = 6

[tools]
named = ["shell", "file_read"]
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert!(def.subagents.is_empty());
    assert!(def.delegate_name.is_none());
}

/// The `delegate_name` field lets an agent expose itself under a
/// shorter / more natural tool name than `delegate_{id}`. For example
/// the `researcher` agent is exposed as `research` in the
/// orchestrator's tool list.
#[test]
fn delegate_name_overrides_default() {
    let toml_src = r#"
id = "researcher"
when_to_use = "Web & docs crawler"
delegate_name = "research"
temperature = 0.4
max_iterations = 8
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert_eq!(def.delegate_name.as_deref(), Some("research"));
}

/// `SkillsWildcard::matches_all` is the predicate the tool builder
/// checks before expanding a wildcard into per-toolkit tools. Only
/// the literal `"*"` should be accepted today — any other pattern
/// (reserved for future specific-toolkit lists) must not match.
#[test]
fn skills_wildcard_only_star_matches_all() {
    let star = SkillsWildcard { skills: "*".into() };
    assert!(star.matches_all());

    let specific = SkillsWildcard {
        skills: "gmail".into(),
    };
    assert!(!specific.matches_all());
}

// ── iteration policy ─────────────────────────────────────────────

#[test]
fn strict_policy_returns_max_iterations_unchanged() {
    let mut def = make_def("summarizer");
    def.max_iterations = 2;
    def.iteration_policy = IterationPolicy::Strict;
    assert_eq!(def.effective_max_iterations(), 2);
}

#[test]
fn extended_policy_raises_cap_to_at_least_extended_constant() {
    let mut def = make_def("code_executor");
    def.max_iterations = 10;
    def.iteration_policy = IterationPolicy::Extended;
    assert_eq!(
        def.effective_max_iterations(),
        super::EXTENDED_MAX_TOOL_ITERATIONS
    );
    assert!(def.effective_max_iterations() > def.max_iterations);
}

#[test]
fn extended_policy_preserves_custom_cap_when_higher_than_constant() {
    let mut def = make_def("custom_agent");
    def.max_iterations = 100;
    def.iteration_policy = IterationPolicy::Extended;
    assert_eq!(def.effective_max_iterations(), 100);
}

#[test]
fn iteration_policy_defaults_to_strict() {
    let def = make_def("test");
    assert_eq!(def.iteration_policy, IterationPolicy::Strict);
}

#[test]
fn iteration_policy_parses_from_toml() {
    let toml_src = r#"
id = "code_executor"
when_to_use = "Runs code"
max_iterations = 10
iteration_policy = "extended"
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert_eq!(def.iteration_policy, IterationPolicy::Extended);
    assert_eq!(
        def.effective_max_iterations(),
        super::EXTENDED_MAX_TOOL_ITERATIONS
    );
}

#[test]
fn iteration_policy_omitted_defaults_strict() {
    let toml_src = r#"
id = "summarizer"
when_to_use = "Summarizes"
max_iterations = 1
"#;
    let def: AgentDefinition = toml::from_str(toml_src).expect("toml parse");
    assert_eq!(def.iteration_policy, IterationPolicy::Strict);
    assert_eq!(def.effective_max_iterations(), 1);
}

// ── Spawn-hierarchy tier rule (validate_tier_transition) ────────────────────
// Single source of truth shared by the boot loader walk and the runtime
// spawn gate in `run_subagent` (issue #4098).

#[test]
fn tier_transition_allows_legal_descending_handoffs() {
    // chat → reasoning, chat → worker, reasoning → worker are the only
    // legal hops; each strictly descends the hierarchy.
    assert!(validate_tier_transition(AgentTier::Chat, AgentTier::Reasoning).is_ok());
    assert!(validate_tier_transition(AgentTier::Chat, AgentTier::Worker).is_ok());
    assert!(validate_tier_transition(AgentTier::Reasoning, AgentTier::Worker).is_ok());
}

#[test]
fn tier_transition_rejects_chat_to_chat() {
    let err = validate_tier_transition(AgentTier::Chat, AgentTier::Chat)
        .expect_err("chat→chat must be rejected");
    // Wording must carry the substrings the loader diagnostics rely on.
    assert!(err.contains("chat") && err.contains("leaf"), "got: {err}");
}

#[test]
fn tier_transition_rejects_reasoning_to_reasoning() {
    let err = validate_tier_transition(AgentTier::Reasoning, AgentTier::Reasoning)
        .expect_err("reasoning→reasoning must be rejected");
    assert!(err.contains("reasoning"), "got: {err}");
}

#[test]
fn tier_transition_allows_upward_reasoning_to_chat() {
    // Upward delegation is intentionally legal: the `subconscious` reasoner
    // hands follow-ups back to the `orchestrator` chat agent. Only same-tier
    // and worker-as-parent hops are forbidden.
    assert!(validate_tier_transition(AgentTier::Reasoning, AgentTier::Chat).is_ok());
}

#[test]
fn tier_transition_rejects_worker_as_parent() {
    // A worker is a leaf executor — it may not spawn any tier, including
    // another worker.
    for child in [AgentTier::Chat, AgentTier::Reasoning, AgentTier::Worker] {
        let err = validate_tier_transition(AgentTier::Worker, child)
            .expect_err("worker must not spawn any tier");
        assert!(
            err.contains("worker") && err.contains("leaf"),
            "worker→{} reason should call out the worker-leaf rule, got: {err}",
            child.as_str()
        );
    }
}

#[test]
fn tier_display_matches_as_str() {
    assert_eq!(AgentTier::Chat.to_string(), "chat");
    assert_eq!(AgentTier::Reasoning.to_string(), "reasoning");
    assert_eq!(AgentTier::Worker.to_string(), "worker");
}

// ── Issue #4868 audit snapshot ──────────────────────────────────────────────
//
// The systemic fix in `build_session_agent_inner` makes every direct-
// invocation call site (flows_build, flows_discover, agent-node runtime,
// cron, MCP server, …) honor each agent's own `effective_max_iterations()`
// instead of the global `config.agent.max_tool_iterations` default (10).
// That changes the *runtime* iteration cap for every agent in the registry —
// some go up (extended policy), some go down (declared max_iterations < 10),
// most stay the same. This test pins the exact expected cap for every
// built-in agent so an accidental `agent.toml` edit (or a new agent landing
// with an unreviewed cap) shows up as a diff here rather than silently
// changing runtime behavior. See PLAN/PR #4868 for the full before/after
// audit table this list mirrors.
#[test]
fn all_builtin_agent_definitions_have_expected_effective_max_iterations() {
    let defs = crate::openhuman::agent_registry::agents::load_builtins()
        .expect("built-in agent TOML must always parse");

    let expected: &[(&str, usize)] = &[
        // Extended policy (or high `max_iterations`) -> effective cap raised.
        ("orchestrator", 15),
        ("code_executor", 50),
        ("context_scout", 50),
        ("integrations_agent", 50),
        ("mcp_agent", 50),
        ("mcp_setup", 50),
        ("planner", 50),
        ("researcher", 50),
        ("skill_creator", 50),
        ("task_manager_agent", 50),
        ("tools_agent", 50),
        // Gated with `flows` (#4797) — absent from a slim build.
        #[cfg(feature = "flows")]
        ("flow_discovery", 50),
        #[cfg(feature = "flows")]
        ("workflow_builder", 50),
        // Compiled out with the `skills` gate — see `openhuman::skills::stub`.
        #[cfg(feature = "skills")]
        ("skill_executor", 50),
        ("tinyplace_agent", 50),
        ("subconscious", 30),
        // Strict policy, declared `max_iterations` below the old global
        // default (10) -> effective cap lowered.
        ("agent_memory", 6),
        ("account_admin_agent", 8),
        ("archivist", 3),
        ("critic", 5),
        ("crypto_agent", 8),
        ("desktop_control_agent", 8),
        ("goals_agent", 5),
        ("help", 6),
        ("image_agent", 8),
        ("markets_agent", 8),
        ("morning_briefing", 8),
        ("profile_memory_agent", 8),
        ("scheduler_agent", 8),
        ("screen_awareness_agent", 8),
        ("settings_agent", 8),
        ("summarizer", 1),
        ("tool_maker", 2),
        ("trigger_reactor", 6),
        ("trigger_triage", 2),
        ("video_agent", 8),
        ("vision_agent", 6),
        // Unchanged.
        ("presentation_agent", 10),
        // Compiled out with the `skills` gate — see `openhuman::skills::stub`.
        #[cfg(feature = "skills")]
        ("skill_setup", 10),
    ];

    for (id, expected_cap) in expected {
        let def = defs
            .iter()
            .find(|d| d.id == *id)
            .unwrap_or_else(|| panic!("missing built-in agent definition: {id}"));
        assert_eq!(
            def.effective_max_iterations(),
            *expected_cap,
            "agent '{id}' effective_max_iterations() mismatch — expected {expected_cap}, got {} \
             (max_iterations={}, iteration_policy={:?}). If this agent's cap was intentionally \
             changed, update this snapshot; otherwise an agent.toml edit just silently changed \
             its runtime iteration budget.",
            def.effective_max_iterations(),
            def.max_iterations,
            def.iteration_policy,
        );
    }

    // Exhaustiveness: every built-in id must appear in the expected list
    // above (and vice versa) so a newly-added agent can't slip in with an
    // unreviewed cap.
    let mut expected_ids: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();
    expected_ids.sort_unstable();
    let mut actual_ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    actual_ids.sort_unstable();
    assert_eq!(
        actual_ids, expected_ids,
        "the set of built-in agent ids changed — add/remove the new agent from this audit \
         snapshot's `expected` list with a deliberate effective_max_iterations() entry"
    );
}
