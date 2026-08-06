//! Tests for the builder module — dedup_visible_tool_specs and related logic.

use super::{
    dedup_visible_tool_specs, ensure_recovery_tool_visible, should_synthesize_delegation_tools,
};
use crate::openhuman::tools::ToolSpec;
use serde_json::json;

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("description for {name}"),
        parameters: json!({}),
    }
}

#[test]
fn recovery_tool_joins_a_named_allowlist() {
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
    use std::collections::HashSet;
    // Empty == "no filter" (all tools visible) AND the deliberately tool-less
    // Named([]) case — both must stay empty so the invariant holds.
    let mut visible: HashSet<String> = HashSet::new();
    ensure_recovery_tool_visible(&mut visible);
    assert!(visible.is_empty(), "empty allowlist must not gain a tool");
}

#[test]
fn drops_duplicates_first_wins() {
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
    let specs = vec![spec("a"), spec("b"), spec("c")];
    let deduped = dedup_visible_tool_specs(specs);
    assert_eq!(deduped.len(), 3);
    assert_eq!(deduped[0].name, "a");
    assert_eq!(deduped[1].name, "b");
    assert_eq!(deduped[2].name, "c");
}

#[test]
fn handles_empty_input() {
    let deduped = dedup_visible_tool_specs(Vec::<ToolSpec>::new());
    assert!(deduped.is_empty());
}

#[test]
fn preserves_full_spec_content_for_kept_entries() {
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

// ─────────────────────────────────────────────────────────────────────────────
// Issue #4868 — `build_session_agent_inner` must resolve the iteration cap
// from the target `AgentDefinition`'s `effective_max_iterations()`, not the
// global `config.agent.max_tool_iterations` default. These tests drive
// `build_session_agent_inner` directly with a hand-picked `target_def`
// (`pub(crate)` for exactly this purpose), independent of the process-global
// `AgentDefinitionRegistry` singleton's init-once state.
// ─────────────────────────────────────────────────────────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> crate::openhuman::config::Config {
    let config = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::openhuman::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

/// Look up a real built-in `AgentDefinition` by id — loaded fresh from the
/// bundled TOML files, entirely independent of the global registry
/// singleton (so tests can't be poisoned by another test's
/// `AgentDefinitionRegistry::init_global*` call, and can't poison later ones).
fn builtin_def(id: &str) -> crate::openhuman::agent::harness::definition::AgentDefinition {
    crate::openhuman::agent::registry::agents::load_builtins()
        .unwrap()
        .into_iter()
        .find(|def| def.id == id)
        .unwrap_or_else(|| panic!("builtin agent definition not found: {id}"))
}

#[tokio::test]
async fn build_session_agent_applies_extended_policy_definition_cap() {
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

    let agent = Agent::build_session_agent_inner(
        &config,
        "code_executor",
        Some(&def),
        None,
        None,
        false,
        None,
    )
    .expect("build_session_agent_inner should succeed for a valid extended-policy definition");

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        50,
        "extended-policy agent must carry its definition's effective cap (50), not the global \
         default (10)"
    );
}

#[tokio::test]
async fn build_session_agent_applies_strict_cap_below_global_default() {
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
        Agent::build_session_agent_inner(&config, "archivist", Some(&def), None, None, false, None)
            .expect("build_session_agent_inner should succeed for a valid strict-low definition");

    assert_eq!(
        agent.agent_config().max_tool_iterations,
        3,
        "strict-policy agent below the global default must still get its own (lower) cap"
    );
}

#[tokio::test]
async fn build_session_agent_falls_back_to_global_default_when_no_definition() {
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    // No `target_def` at all (e.g. registry not yet initialised, or a
    // legacy caller that never resolved one) — must fall back to the
    // unmodified global `config.agent.max_tool_iterations`.
    let agent =
        Agent::build_session_agent_inner(&config, "orchestrator", None, None, None, false, None)
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
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let orchestrator = builtin_def("orchestrator");
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".to_string();
    profile.built_in = false;
    profile.allowed_tools = Some(vec!["file_read".to_string()]);

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build profile-scoped session");

    assert_eq!(
        agent.visible_tool_names_for_test(),
        &["file_read".to_string()].into_iter().collect(),
        "every profile-aware caller must inherit the same tool restriction"
    );
    assert_eq!(
        agent.subagent_tool_ceiling_names_for_test(),
        &["file_read".to_string()].into_iter().collect(),
        "an explicit profile tool restriction must also ceiling delegated agents"
    );
}

#[tokio::test]
async fn channel_ceiling_does_not_inherit_orchestrator_role_visibility() {
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config
        .agent
        .channel_permissions
        .insert("internal".to_string(), "execute".to_string());
    let def = builtin_def("orchestrator");

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&def),
        None,
        None,
        false,
        None,
    )
    .expect("build channel-scoped orchestrator session");

    assert!(
        !agent.visible_tool_names_for_test().contains("file_write"),
        "the orchestrator must not advertise the specialist file_write tool"
    );
    assert!(
        agent
            .subagent_tool_ceiling_names_for_test()
            .contains("file_write"),
        "an execute-capable channel must let code_executor inherit file_write"
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
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // The profile-less path (the legacy default) must stay byte-identical:
    // no active profile id is stamped.
    let agent =
        Agent::build_session_agent_inner(&config, "orchestrator", None, None, None, false, None)
            .expect("build_session_agent_inner with no profile should succeed");

    assert_eq!(
        agent.active_profile_id, None,
        "the profile-less session must not carry an active profile id"
    );
}

// ── Finding #1 (Codex): dedicated memory subtree on the ordinary session path ─

/// Build a non-default profile with the given id + dedicated-memory flag.
fn custom_profile(
    id: &str,
    dedicated_memory: bool,
) -> crate::openhuman::agent::profiles::AgentProfile {
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = id.to_string();
    profile.name = id.to_string();
    profile.built_in = false;
    profile.is_master = false;
    profile.memory_dir_suffix = None;
    profile.dedicated_memory = dedicated_memory;
    profile
}

#[tokio::test]
async fn build_session_agent_routes_dedicated_memory_to_profile_subtree() {
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let profile = custom_profile("alice", true);

    let _agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build_session_agent_inner with a dedicated-memory profile should succeed");

    // The session's capture/recall store (UnifiedMemory) is rooted at
    // `<workspace>/memory-alice`, not the shared `memory/` tree.
    assert!(
        config
            .workspace_dir
            .join("memory-alice")
            .join("memory.db")
            .exists(),
        "a dedicatedMemory profile must route session memory to memory-<id>"
    );
}

#[tokio::test]
async fn build_session_agent_profile_less_uses_shared_memory_subtree() {
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Profile-less path stays byte-identical: session memory uses the shared
    // `memory/` subtree, and no per-profile subtree is created.
    let _agent =
        Agent::build_session_agent_inner(&config, "orchestrator", None, None, None, false, None)
            .expect("build_session_agent_inner without a profile should succeed");

    assert!(
        config
            .workspace_dir
            .join("memory")
            .join("memory.db")
            .exists(),
        "the profile-less session must use the shared memory subtree"
    );
    assert!(
        !config.workspace_dir.join("memory-alice").exists(),
        "no per-profile memory subtree should exist for a profile-less session"
    );
}

// ── Finding #2 (Codex): profile SOUL.md injected into the live session prompt ─

#[tokio::test]
async fn build_session_agent_injects_profile_soul_into_prompt() {
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

#[tokio::test]
async fn build_session_agent_injects_default_profile_soul_into_prompt() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let home = config.workspace_dir.join("personalities").join("default");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("SOUL.md"),
        "I am the user-edited Default profile identity.",
    )
    .unwrap();

    let profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build default-profile session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        prompt.contains("I am the user-edited Default profile identity."),
        "the live Default profile prompt must include personalities/default/SOUL.md"
    );
}

#[tokio::test]
async fn build_session_agent_profile_less_prompt_has_no_personality_soul() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A personalities/alice/SOUL.md exists on disk, but a profile-less session
    // must never pull it — the prompt stays byte-identical to today.
    let home = config.workspace_dir.join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "I am Alice, a meticulous archivist.").unwrap();

    let agent =
        Agent::build_session_agent_inner(&config, "orchestrator", None, None, None, false, None)
            .expect("build_session_agent_inner without a profile should succeed");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        !prompt.contains("I am Alice, a meticulous archivist."),
        "a profile-less session must not inject any profile SOUL.md"
    );
}

#[tokio::test]
async fn from_config_for_agent_still_errors_for_a_genuinely_unknown_id() {
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // No harness definition AND no config.agent_registry entry for this id —
    // the factory must still hard-error rather than silently building an
    // unfiltered/legacy agent.
    //
    // Note: `Agent` intentionally has no `Debug` impl (it holds `Box<dyn
    // Tool>` / provider trait objects), so this must use `match` +
    // `.is_err()` rather than `.expect_err()`, which requires `T: Debug`.
    let result = Agent::from_config_for_agent(&config, "totally_unknown_agent_id_b38");
    assert!(
        result.is_err(),
        "an id with no harness definition and no custom entry must error"
    );
    let err = result.err().unwrap();
    assert!(
        err.to_string().contains("totally_unknown_agent_id_b38"),
        "error should name the unresolved agent id: {err}"
    );
}
