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
    use crate::openhuman::tokenjuice::RETRIEVE_TOOL_NAME as RECOVERY_TOOL_NAME;
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
    let defs = crate::openhuman::agent_registry::agents::load_builtins().unwrap();
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
    crate::openhuman::agent_registry::agents::load_builtins()
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

// ── #5050 Fix 1: shared `Arc<Config>` for the per-build tool config ──────────

#[test]
fn tool_config_shares_base_arc_when_ui_control_toggle_off() {
    use super::factory::resolve_tool_config;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.computer_control.ax_interact_mutations = false;
    let base = Arc::new(cfg);

    // No enabled tools → the App-UI-Control toggle does not fire → the tool
    // registry shares the base `Arc` (a refcount bump), not a deep clone.
    let resolved = resolve_tool_config(&base, &[]);
    assert!(
        Arc::ptr_eq(&base, &resolved),
        "toggle off must reuse the base config Arc rather than deep-clone it"
    );
}

#[test]
fn tool_config_grant_is_scoped_and_leaves_base_untouched() {
    use super::factory::resolve_tool_config;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.computer_control.ax_interact_mutations = false;
    let base = Arc::new(cfg);

    // Enabling `ax_interact` fires the toggle: the tool registry gets the mutation
    // grant, but as a *distinct* instance — the base config (which feeds the turn
    // provider + reflection hook) must stay ungranted so the grant cannot leak.
    let resolved = resolve_tool_config(&base, &["ax_interact".to_string()]);
    assert!(
        resolved.computer_control.ax_interact_mutations,
        "the tool-registry config must carry the granted mutation flag"
    );
    assert!(
        !Arc::ptr_eq(&base, &resolved),
        "granting must produce a distinct config, not alias the shared base"
    );
    assert!(
        !base.computer_control.ax_interact_mutations,
        "the base config must stay ungranted — the grant is scoped to the tool registry"
    );
}

#[test]
fn tool_config_reuses_base_when_mutations_already_granted_globally() {
    use super::factory::resolve_tool_config;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.computer_control.ax_interact_mutations = true;
    let base = Arc::new(cfg);

    // Already granted globally (e.g. Full autonomy) → no clone even when the tool
    // is enabled, since there is nothing to grant.
    let resolved = resolve_tool_config(&base, &["ax_interact".to_string()]);
    assert!(
        Arc::ptr_eq(&base, &resolved),
        "an already-granted base config must not be re-cloned"
    );
}
