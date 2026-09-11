use super::*;

#[test]
fn recovery_tool_joins_a_named_allowlist() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::inference::tokenjuice::RETRIEVE_TOOL_NAME as RECOVERY_TOOL_NAME;
    use std::collections::HashSet;

    // A curated Named-scope allowlist gains retrieve_tool_output as a *real*
    // member, so the policy session, advertised specs, and the run-time
    // visible-name gate (all driven by this set) make a compaction footer
    // actionable.
    let mut visible: HashSet<String> = ["file_read".to_string(), "grep".to_string()]
        .into_iter()
        .collect();
    ensure_recovery_tool_visible(&mut visible);
    assert!(
        visible.contains(RECOVERY_TOOL_NAME),
        "recovery tool must join: {visible:?}"
    );
    assert!(visible.contains("file_read"));
}

#[test]
fn empty_allowlist_stays_empty() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use std::collections::HashSet;
    // Empty == "no filter" (all tools visible) AND the deliberately tool-less
    // Named([]) case — both must stay empty so the invariant holds.
    let mut visible: HashSet<String> = HashSet::new();
    ensure_recovery_tool_visible(&mut visible);
    assert!(visible.is_empty(), "empty allowlist must not gain a tool");
}

#[test]
fn drops_duplicates_first_wins() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    // Real-world collision: researcher's `delegate_name = "research"`
    // synthesises a delegate tool that shadows a same-named skill.
    // Anthropic 400s on duplicate tool names; the dedup helper must
    // keep the *first* occurrence so registration order semantics
    // are preserved (the underlying tool dispatch lookup-by-name
    // still resolves the right tool).
    let specs = vec![
        spec("research"), // skill
        spec("plan"),
        spec("research"), // delegate, dropped
        spec("run_code"),
        spec("plan"), // dropped
    ];

    let deduped = dedup_visible_tool_specs(specs);

    let names: Vec<&str> = deduped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["research", "plan", "run_code"]);
}

#[test]
fn passes_through_when_no_duplicates() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    let specs = vec![spec("a"), spec("b"), spec("c")];
    let deduped = dedup_visible_tool_specs(specs);
    assert_eq!(deduped.len(), 3);
    assert_eq!(deduped[0].name, "a");
    assert_eq!(deduped[1].name, "b");
    assert_eq!(deduped[2].name, "c");
}

#[test]
fn handles_empty_input() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    let deduped = dedup_visible_tool_specs(Vec::<ToolSpec>::new());
    assert!(deduped.is_empty());
}

#[test]
fn preserves_full_spec_content_for_kept_entries() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    // Description + parameters must survive the dedup pass intact —
    // the LLM uses both for tool-call decisions, and corrupting them
    // would silently degrade function-calling quality.
    let mut spec_a = spec("alpha");
    spec_a.description = "first alpha — should win".to_string();
    spec_a.parameters = json!({"type": "object", "required": ["x"]});

    let mut spec_a_dup = spec("alpha");
    spec_a_dup.description = "second alpha — should be dropped".to_string();

    let deduped = dedup_visible_tool_specs(vec![spec_a.clone(), spec_a_dup]);

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].description, "first alpha — should win");
    assert_eq!(
        deduped[0].parameters,
        json!({"type": "object", "required": ["x"]})
    );
}

#[test]
fn automatic_memory_policy_does_not_synthesize_delegate_tools() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    let defs = crate::openhuman::agent::registry::agents::load_builtins().unwrap();
    let help = defs
        .iter()
        .find(|def| def.id == "help")
        .expect("help agent is built in");
    let orchestrator = defs
        .iter()
        .find(|def| def.id == "orchestrator")
        .expect("orchestrator is built in");

    assert!(
        !should_synthesize_delegation_tools(help),
        "automatic memory policy should not add delegate tools"
    );
    assert!(
        should_synthesize_delegation_tools(orchestrator),
        "orchestrator still needs synthesized delegate tools"
    );
}

#[tokio::test]
async fn build_session_agent_applies_extended_policy_definition_cap() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(
        config.agent.max_tool_iterations, 10,
        "precondition: global default must be 10 for this test to distinguish the two"
    );

    // `code_executor` declares `iteration_policy = "extended"` with
    // `max_iterations = 10` in its agent.toml, so its effective cap is
    // `EXTENDED_MAX_TOOL_ITERATIONS` (50) — not the raw `max_iterations`.
    let def = builtin_def("code_executor");
    assert_eq!(def.effective_max_iterations(), 50);

    let agent =
        Agent::build_session_agent_inner(&config, "code_executor", Some(&def), None, false, None)
            .expect(
                "build_session_agent_inner should succeed for a valid extended-policy definition",
            );

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        50,
        "extended-policy agent must carry its definition's effective cap (50), not the global \
         default (10)"
    );
}

#[tokio::test]
async fn build_session_agent_applies_strict_cap_below_global_default() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    // `archivist` is strict-policy with a declared `max_iterations = 3` —
    // well below the global default of 10. The definition cap must still
    // win (lowering the runtime cap), not just raise it.
    let def = builtin_def("archivist");
    assert_eq!(def.effective_max_iterations(), 3);

    let agent =
        Agent::build_session_agent_inner(&config, "archivist", Some(&def), None, false, None)
            .expect("build_session_agent_inner should succeed for a valid strict-low definition");

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        3,
        "strict-policy agent below the global default must still get its own (lower) cap"
    );
}

#[tokio::test]
async fn build_session_agent_falls_back_to_global_default_when_no_definition() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    // No `target_def` at all (e.g. registry not yet initialised, or a
    // legacy caller that never resolved one) — must fall back to the
    // unmodified global `config.agent.max_tool_iterations`.
    let agent = Agent::build_session_agent_inner(&config, "orchestrator", None, None, false, None)
        .expect("build_session_agent_inner should succeed with no definition");

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        10,
        "with no definition, the global config default must be used unchanged"
    );
}

// ── 1a: active profile id plumbed onto the built session ─────────────────────

#[tokio::test]
async fn build_session_agent_carries_active_profile_id_when_profile_present() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".to_string();
    profile.built_in = false;
    profile.is_master = false;

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build_session_agent_inner with a profile should succeed");

    assert_eq!(
        agent.active_profile_id.as_deref(),
        Some("alice"),
        "an active profile must plumb its id onto the built session"
    );
}

#[tokio::test]
async fn profile_allowed_tools_restrict_shared_session_builder() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let orchestrator = builtin_def("orchestrator");
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".to_string();
    profile.built_in = false;
    // `shell` rather than `file_read`: this test is about a profile's
    // `allowed_tools` reaching every caller, and `file_read` moved into the
    // `files` tool pack, so the visible set would come back as the `use_skill`
    // proxy and the assertion would be about packing instead.
    profile.allowed_tools = Some(vec!["shell".to_string()]);

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        false,
        Some(&profile),
    )
    .expect("build profile-scoped session");

    assert_eq!(
        agent.visible_tool_names_for_test(),
        &["shell".to_string()].into_iter().collect(),
        "every profile-aware caller must inherit the same tool restriction"
    );
    assert_eq!(
        agent.subagent_tool_ceiling_names_for_test(),
        &["shell".to_string()].into_iter().collect(),
        "an explicit profile tool restriction must also ceiling delegated agents"
    );
}

#[tokio::test]
async fn channel_ceiling_does_not_inherit_orchestrator_role_visibility() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config
        .agent
        .channel_permissions
        .insert("internal".to_string(), "execute".to_string());
    let def = builtin_def("orchestrator");

    let agent =
        Agent::build_session_agent_inner(&config, "orchestrator", Some(&def), None, false, None)
            .expect("build channel-scoped orchestrator session");

    // Withheld from the parent: `shell` covers reading and writing a file, so
    // the `files` pack takes the dedicated tools off the Master Agent's wire.
    assert!(
        !agent.visible_tool_names_for_test().contains("file_write"),
        "`file_write` belongs to the `files` pack and must not be advertised \
         to the Master Agent, which reaches it through `use_skill`"
    );
    // …and that withholding must NOT travel down. The ceiling is built from
    // the full spec list against the channel policy, deliberately independent
    // of pack disclosure, so `code_executor` — which owns the pack — still
    // inherits the real tool. A ceiling computed from the parent's advertised
    // set instead would silently strip every packed tool from every child,
    // which is the regression this half exists to catch.
    assert!(
        agent
            .subagent_tool_ceiling_names_for_test()
            .contains("file_write"),
        "an execute-capable channel must let code_executor inherit file_write \
         even though the parent no longer advertises it"
    );
    assert!(
        !agent
            .subagent_tool_ceiling_names_for_test()
            .contains("update_apply"),
        "the execute channel ceiling must still exclude dangerous tools"
    );
}

#[tokio::test]
async fn dedicated_memory_profile_scopes_tree_and_transcript_storage() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".to_string();
    profile.built_in = false;
    profile.dedicated_memory = true;

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build dedicated-memory session");

    assert_eq!(agent.memory_subdir, "memory-alice");
    assert_eq!(agent.session_raw_subdir, "session_raw-alice");
}

#[tokio::test]
async fn build_session_agent_leaves_active_profile_id_none_without_profile() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // The profile-less path (the legacy default) must stay byte-identical:
    // no active profile id is stamped.
    let agent = Agent::build_session_agent_inner(&config, "orchestrator", None, None, false, None)
        .expect("build_session_agent_inner with no profile should succeed");

    assert_eq!(
        agent.active_profile_id, None,
        "the profile-less session must not carry an active profile id"
    );
}

#[tokio::test]
async fn build_session_agent_routes_dedicated_memory_to_profile_subtree() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let profile = custom_profile("alice", true);

    let _agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build_session_agent_inner with a dedicated-memory profile should succeed");

    // Asserted on the binding, not on a file. Session memory is bound through
    // `DriverMemory::for_subtree`, and a module-backed driver opens its subtree
    // lazily on the first `OpenStore` call — so building an agent puts nothing
    // on disk, and the one call that would is a module round trip that times
    // out in a unit test. The binding knows the answer at bind time, which is
    // where the routing decision is actually made.
    let bound = crate::openhuman::memory::binding::for_subtree(
        &config.workspace_dir,
        "memory-alice",
        &config.subsystems.memory,
    )
    .expect("the dedicated subtree must be bound");
    assert_eq!(
        bound.memory_subdir(),
        "memory-alice",
        "a dedicatedMemory profile must route session memory to memory-<id>"
    );
    assert_ne!(
        bound.memory_subdir(),
        "memory",
        "a dedicatedMemory profile must not share the common memory subtree"
    );
}

#[tokio::test]
async fn build_session_agent_profile_less_uses_shared_memory_subtree() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Profile-less path stays byte-identical: session memory uses the shared
    // `memory/` subtree, and no per-profile subtree is created.
    let _agent = Agent::build_session_agent_inner(&config, "orchestrator", None, None, false, None)
        .expect("build_session_agent_inner without a profile should succeed");

    // Same reasoning as the dedicated-memory case above: the routing decision
    // lives on the binding, and nothing reaches disk until memory is used.
    let bound = crate::openhuman::memory::binding::for_subtree(
        &config.workspace_dir,
        "memory",
        &config.subsystems.memory,
    )
    .expect("the shared subtree must be bound");
    assert_eq!(
        bound.memory_subdir(),
        "memory",
        "the profile-less session must use the shared memory subtree"
    );
    // The companion "and no per-profile subtree exists" check is deliberately
    // gone rather than repointed. It asserted the ABSENCE of a directory that a
    // module-backed driver no longer creates for anyone at bind time, so it
    // passed whatever the routing did — vacuous, and worse than nothing because
    // it read like coverage. The positive assertion above is the whole property
    // this test can honestly make.
}

// ── Finding #2 (Codex): profile SOUL.md injected into the live session prompt ─

#[tokio::test]
async fn build_session_agent_injects_profile_soul_into_prompt() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::write(
        config.workspace_dir.join("SOUL.md"),
        "I am the conflicting workspace-root identity.",
    )
    .unwrap();
    // Seed the non-default profile's home SOUL.md (as ensure_profile_home would).
    let home = config.workspace_dir.join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "I am Alice, a meticulous archivist.").unwrap();

    let profile = custom_profile("alice", false);
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build_session_agent_inner with a profile should succeed");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        prompt.contains("I am Alice, a meticulous archivist."),
        "the live profile session prompt must include the profile SOUL.md content"
    );
    assert!(
        !prompt.contains("I am the conflicting workspace-root identity."),
        "profile SOUL.md must replace, not accompany, workspace-root SOUL.md"
    );
}

#[tokio::test]
async fn build_session_agent_uses_profile_memory_instead_of_root_memory() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::write(
        config.workspace_dir.join("MEMORY.md"),
        "shared root memory marker",
    )
    .unwrap();
    let home = config.workspace_dir.join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("MEMORY.md"), "alice private memory marker").unwrap();

    let profile = custom_profile("alice", false);
    let orchestrator = builtin_def("orchestrator");
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        false,
        Some(&profile),
    )
    .expect("build profile session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(prompt.contains("alice private memory marker"));
    assert!(!prompt.contains("shared root memory marker"));
}

/// #6040 — the memory-access instruction is about the memory tools, not the
/// learning subsystem, so it must be in the prompt with `learning.enabled`
/// off (the default) whenever a retrieval tool is registered and visible.
///
/// Passes the definition explicitly via [`builtin_def`] rather than letting the
/// factory resolve `"orchestrator"` from the registry, and that is load-bearing
/// rather than ceremony.
///
/// The section is gated on `memory_recall` being **registered and visible after
/// tool filtering** (`any_tool_offered`), and the visible set comes from the
/// resolved definition's tool scope. With `None` here the factory reads
/// `AgentDefinitionRegistry`'s `static GLOBAL: OnceLock<…>`
/// (`harness/definition_part_02.rs:24`) — first-write-wins and never reset — so
/// the test was asserting against whichever definition set some *other* test in
/// the binary had installed first. That is exactly the hazard `builtin_def`
/// was written for: it loads fresh from the bundled TOML, "entirely independent
/// of the global registry singleton".
///
/// It is why this passed run alone and failed inside the full
/// `openhuman::agent` run (`ci-lite` scopes the Rust lane per changed domain,
/// so the whole scope only runs when a PR touches `agent/`), and why the
/// sibling write-side test in `builder_tests_part_03_tests.rs` never flaked —
/// it already supplied `builtin_def("orchestrator")`.
#[tokio::test]
async fn memory_access_instruction_is_present_with_learning_disabled() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;
    use crate::openhuman::agent::learning::MEMORY_ACCESS_INSTRUCTION;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.learning.enabled = false;

    let orchestrator = builtin_def("orchestrator");
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        false,
        None,
    )
    .expect("build session agent");
    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");

    assert!(
        prompt.contains(MEMORY_ACCESS_INSTRUCTION.trim()),
        "the memory-access section must not be gated on learning.enabled"
    );
    assert!(
        prompt.contains("Never say something is not stored"),
        "the instruction must forbid claiming absence without a retrieval"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// B38 (Gap 2) — a custom (non-shipped) `AgentRegistryEntry` must synthesize a
// real `AgentDefinition` and run with its own `ToolScope::Named` filter,
// instead of the factory hard-erroring "agent definition '…' not found in
// registry" (chat / task-dispatcher) because it never consulted
// `config.agent_registry.entries`.
//
// Regression note: this test deliberately does NOT call
// `AgentDefinitionRegistry::init_global*` itself, so — depending on whether
// an earlier test in this binary already initialised the process-wide
// `OnceLock` singleton — it exercises `build_session_agent_inner`'s tool-
// visibility computation under EITHER state: `(Some(def), Some(registry))`
// or `(Some(def), None)`. Both arms must apply `def.tools` (the synthesized
// `ToolScope::Named` from `definition_from_registry_entry`); the `None`
// (registry-uninitialized) arm previously fell through to the catch-all
// "no registry, no filter" case and silently discarded the custom agent's
// allowlist, leaving `visible_tool_names_for_test()` empty. See the
// `(Some(def), None)` match arm in `factory.rs`'s delegation-tool-and-
// visibility block for the fix.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn from_config_for_agent_synthesizes_custom_registry_entry_with_named_scope() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;
    use crate::openhuman::agent::registry::types::{
        AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy,
    };
    use crate::openhuman::inference::tokenjuice::RETRIEVE_TOOL_NAME;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.agent_registry.entries = vec![AgentRegistryEntry {
        id: "finance_analyst_b38".to_string(),
        name: "Finance Analyst".to_string(),
        description: "Reviews spend and drafts finance summaries.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: Some("hint:reasoning".to_string()),
        system_prompt: Some("You are a meticulous finance analyst.".to_string()),
        tool_allowlist: vec!["memory_search".to_string(), "web_search".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::default(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }];

    // Precondition: this id must NOT be a harness definition (built-in or
    // workspace TOML) — the whole point is that only the config-backed
    // custom registry knows about it.
    assert!(
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .map(|reg| reg.get("finance_analyst_b38").is_none())
            .unwrap_or(true),
        "test id must not collide with a real harness definition"
    );

    let agent = Agent::from_config_for_agent(&config, "finance_analyst_b38").expect(
        "a custom agent_registry entry must synthesize a real AgentDefinition and build \
         successfully instead of erroring",
    );

    let visible = agent.visible_tool_names_for_test();
    assert!(
        visible.contains("memory_search") && visible.contains("web_search"),
        "the custom agent's tool_allowlist must become a real ToolScope::Named filter: {visible:?}"
    );
    assert!(
        visible.contains(RETRIEVE_TOOL_NAME),
        "the compaction recovery tool must join any non-empty Named allowlist: {visible:?}"
    );
    assert!(
        !visible.contains("automate"),
        "a tool outside the custom agent's allowlist must not be visible: {visible:?}"
    );
}

/// `from_config` hands the build-time delegation tools to the builder's
/// synthesised set rather than folding them into the durable registry.
///
/// Before #6145 the factory appended them to `tools`, so the first
/// `refresh_delegation_tools` — which replaces `Agent::synthesized_tools` and
/// never touches `tools` — would have left the build-time instances behind:
/// duplicated in the prompt catalogue next to their fresh replacements, and
/// only kept off the dispatch path by set ordering.
#[test]
fn from_config_keeps_build_time_delegation_tools_out_of_the_durable_registry() {
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let agent = crate::openhuman::agent::Agent::from_config_for_agent(&config, "orchestrator")
        .expect("orchestrator session build");

    let synthesized: Vec<String> = agent
        .synthesized_tools_arc()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert!(
        !synthesized.is_empty(),
        "the orchestrator declares sub-agents, so the factory must synthesise delegates"
    );
    for name in &synthesized {
        assert!(
            agent.tools().iter().all(|tool| tool.name() != name),
            "build-time delegate `{name}` must not also sit in the durable registry"
        );
        assert!(
            agent.tool_specs().iter().any(|spec| &spec.name == name),
            "build-time delegate `{name}` must be advertised"
        );
        assert!(
            agent.tool_policy_session.decisions.contains_key(name),
            "build-time delegate `{name}` must carry a policy decision"
        );
    }
    let expected_mask: std::collections::HashSet<String> = synthesized.iter().cloned().collect();
    assert_eq!(
        agent.synthesized_tool_names, expected_mask,
        "the refresh mask must be seeded with exactly the synthesised names"
    );
    // What a sub-agent is handed: the durable registry and its specs, index
    // for index, with no synthesised delegate among them.
    let durable_specs = agent.durable_tool_specs_arc();
    assert_eq!(durable_specs.len(), agent.tools().len());
    for (tool, spec) in agent.tools().iter().zip(durable_specs.iter()) {
        assert_eq!(
            tool.name(),
            spec.name,
            "durable specs must track the registry index for index"
        );
    }
}
