//! Tests for the builder module — dedup_visible_tool_specs and related logic.

use super::{
    dedup_visible_tool_specs, ensure_recovery_tool_visible, should_synthesize_delegation_tools,
    visible_tool_specs_for_policy,
};
use serde_json::json;
use tinytools::ToolSpec;

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("description for {name}"),
        parameters: json!({}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Issue #4868 — `build_session_agent_inner` must resolve the iteration cap
// from the target `AgentDefinition`'s `effective_max_iterations()`, not the
// global `config.agent.max_tool_iterations` default. These tests drive
// `build_session_agent_inner` directly with a hand-picked `target_def`
// (`pub(crate)` for exactly this purpose), independent of the process-global
// `AgentDefinitionRegistry` singleton's init-once state.
// ─────────────────────────────────────────────────────────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> crate::config::Config {
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

/// Look up a real built-in `AgentDefinition` by id — loaded fresh from the
/// bundled TOML files, entirely independent of the global registry
/// singleton (so tests can't be poisoned by another test's
/// `AgentDefinitionRegistry::init_global*` call, and can't poison later ones).
fn builtin_def(id: &str) -> crate::agent::harness::definition::AgentDefinition {
    crate::agent::registry::agents::load_builtins()
        .unwrap()
        .into_iter()
        .find(|def| def.id == id)
        .unwrap_or_else(|| panic!("builtin agent definition not found: {id}"))
}

#[path = "builder_tests_explicit_definition_tests.rs"]
mod explicit_definition_tests;
#[path = "builder_tests_memory_write_instruction_tests.rs"]
mod memory_write_instruction_tests;
#[path = "builder_tests_tool_exposure_tests.rs"]
mod tool_exposure_tests;
#[path = "builder_tests_tool_spec_views_tests.rs"]
mod tool_spec_views_tests;

// ── use_skill's advertised spec is scoped to the session ────────────────────

use crate::tools::agent_policy::{
    TaskProfile, TaskRiskLevel, ToolPolicyAction, ToolPolicyDecision, ToolPolicySession,
};
use tinytools::PermissionLevel;

fn session_allowing(names: &[&str]) -> ToolPolicySession {
    ToolPolicySession {
        profile: TaskProfile {
            agent_id: "orchestrator".to_string(),
            channel: "web_chat".to_string(),
            entrypoint: "chat".to_string(),
            risk_level: TaskRiskLevel::Low,
            allowed_permission: PermissionLevel::Dangerous,
        },
        capabilities: vec![],
        allowed_tool_names: names.iter().map(|n| n.to_string()).collect(),
        blocked_tool_names: Default::default(),
        hidden_tool_names: Default::default(),
        decisions: names
            .iter()
            .map(|n| {
                (
                    n.to_string(),
                    ToolPolicyDecision {
                        tool_name: n.to_string(),
                        action: ToolPolicyAction::Allow,
                        required_permission: None,
                        allowed_permission: PermissionLevel::Dangerous,
                    },
                )
            })
            .collect(),
    }
}

fn use_skill_spec_from_registry() -> ToolSpec {
    let mut tools: Vec<Box<dyn tinytools::Tool>> = Vec::new();
    crate::tools::toolpacks::append_pack_tools(&mut tools);
    let tool = tools
        .iter()
        .find(|t| t.name() == crate::tools::toolpacks::USE_SKILL)
        .expect("append_pack_tools registers use_skill");
    ToolSpec {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.parameters_schema(),
    }
}

/// Proves the scoping is actually WIRED, not merely available. A correct helper
/// nobody calls advertises every pack exactly as before.
#[test]
fn visible_specs_scope_use_skills_index_to_the_session() {
    // `Arc` leaves, because the three spec views share them — the assertions
    // below are unchanged, only the carrier is.
    let specs: Vec<std::sync::Arc<ToolSpec>> =
        vec![std::sync::Arc::new(use_skill_spec_from_registry())];
    let visible: std::collections::HashSet<String> = specs.iter().map(|s| s.name.clone()).collect();
    // Reachable: one workflows tool. Everything else in every other pack is
    // denied, exactly like the orchestrator against `system` / `audio`.
    let session = session_allowing(&["run_workflow", crate::tools::toolpacks::USE_SKILL]);

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    let load = out
        .iter()
        .find(|s| s.name == crate::tools::toolpacks::USE_SKILL)
        .expect("use_skill is still offered — workflows is reachable");

    assert!(
        load.description.contains("`workflows`"),
        "the reachable pack must survive: {}",
        load.description
    );
    assert!(
        !load.description.contains("`system`"),
        "a pack with nothing callable must not reach the wire: {}",
        load.description
    );
    let ids = load
        .parameters
        .pointer("/properties/skill/enum")
        .and_then(|v| v.as_array())
        .expect("skill enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["workflows"], "the enum is scoped too");
}

/// A session that can reach no pack at all should not carry the pack tool.
#[test]
fn visible_specs_drop_the_pack_tool_when_no_pack_is_reachable() {
    // `Arc` leaves, because the three spec views share them — the assertions
    // below are unchanged, only the carrier is.
    let specs: Vec<std::sync::Arc<ToolSpec>> =
        vec![std::sync::Arc::new(use_skill_spec_from_registry())];
    let visible: std::collections::HashSet<String> = specs.iter().map(|s| s.name.clone()).collect();
    let session = session_allowing(&[crate::tools::toolpacks::USE_SKILL]);

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    assert!(
        out.is_empty(),
        "with no reachable pack, the pack tool does not earn its schema: {:?}",
        out.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

/// **The regression Codex caught on #6215, pinned with a realistic session.**
///
/// My first tests hand-built a `ToolPolicySession` whose `decisions` were
/// explicit `Allow`s — a fixture that cannot express the state every packed
/// tool is actually in. The real harness strips packed names from `visible`,
/// which classifies them `HideFromPrompt`, and `is_denied()` answers true for
/// that. A predicate built on `is_denied` therefore reported *no* pack as
/// reachable and dropped `use_skill` from the wire — deleting
/// the only route to every withheld tool.
#[test]
fn a_realistic_withheld_session_keeps_its_packs_advertised() {
    use crate::tools::agent_policy::ToolPolicyEngine;
    use crate::tools::toolpacks::{append_pack_tools, strip_packed_from_visible};

    struct Fake(&'static str);
    #[async_trait::async_trait]
    impl tinytools::Tool for Fake {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
            Ok(tinytools::ToolResult::success("ok"))
        }
    }

    let mut tools: Vec<Box<dyn tinytools::Tool>> =
        vec![Box::new(Fake("goal_set")), Box::new(Fake("goal_get"))];
    append_pack_tools(&mut tools);

    let mut visible: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name().to_string()).collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert!(
        !visible.contains("goal_set"),
        "precondition: the pack is withheld from the prompt"
    );

    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web_chat",
        "chat",
        &Default::default(),
        &tools,
        &visible,
    );

    let specs: Vec<std::sync::Arc<ToolSpec>> = tools
        .iter()
        .map(|t| {
            std::sync::Arc::new(ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
        })
        .collect();

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();

    assert!(
        names.contains(&crate::tools::toolpacks::USE_SKILL),
        "use_skill must survive — the pack it opens is reachable: {names:?}"
    );
    let load = out
        .iter()
        .find(|s| s.name == crate::tools::toolpacks::USE_SKILL)
        .expect("use_skill spec");
    assert!(
        load.description.contains("`goals`"),
        "the withheld-but-callable pack must still be advertised: {}",
        load.description
    );
}

/// **Listing visibility must not ride on the named-call permission ceiling.**
///
/// `UseSkillTool::permission_level()` reports the MAX over every packed
/// tool — correct for a *named* call, where the worst case genuinely is the
/// most dangerous packed tool. But `tool_policy.is_allowed(&spec.name)` is
/// built from that same argument-less ceiling, so a session excluding
/// `use_skill` from `allowed_tool_names` (exactly what the real engine does
/// when one packed tool exceeds the channel's permission ceiling) must not
/// erase the whole proxy — `use_skill`'s own listing action
/// (`skill` alone, no `tool`) is always `ReadOnly` per
/// `permission_level_with_args`, and every OTHER pack's tools stay reachable
/// through it regardless of what one dangerous tool in some other pack needs.
#[test]
fn use_skill_survives_a_ceiling_that_excludes_it_when_a_pack_is_still_reachable() {
    // `allowed_tool_names` deliberately omits `USE_SKILL` itself — simulating
    // the real engine having excluded it because *some* packed tool (not
    // `run_workflow`) exceeded the channel's permission ceiling.
    let session = session_allowing(&["run_workflow"]);
    assert!(
        !session
            .allowed_tool_names
            .contains(crate::tools::toolpacks::USE_SKILL),
        "precondition: use_skill itself is not in the allowlist"
    );

    let specs: Vec<std::sync::Arc<ToolSpec>> =
        vec![std::sync::Arc::new(use_skill_spec_from_registry())];
    let visible: std::collections::HashSet<String> = specs.iter().map(|s| s.name.clone()).collect();

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    let load = out
        .iter()
        .find(|s| s.name == crate::tools::toolpacks::USE_SKILL)
        .expect(
            "use_skill must survive even though it is not itself in allowed_tool_names — \
             its listing action is always ReadOnly and the workflows pack is reachable",
        );
    assert!(
        load.description.contains("`workflows`"),
        "the reachable pack must still be advertised: {}",
        load.description
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// `named = []` means zero tools, not every tool.
//
// The harness's visible-tool set uses empty as its "no filter" sentinel, so an
// agent declaring an empty named scope was handed the entire registry — the
// exact opposite of what it asked for. `summarizer` and `trigger_triage` both
// declare `named = []` in their shipped `agent.toml`, and both were carrying
// 109 tools / 82,986 B of schema apiece: 18% of the fleet's whole fixed prefix,
// on the two agents that had asked for none. `trigger_triage`'s own comment
// says local 1B-class models are unreliable at nested tool calls, "so we keep
// the turn flat" — so this was not only waste, it was actively working against
// the thing the author documented.
//
// `NO_TOOLS_SENTINEL` is how an empty belt survives a set whose empty state is
// already spoken for.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_empty_named_scope_advertises_no_tools_at_all() {
    use crate::agent::session_host::types::OpenHumanSessionHost;

    // `summarizer` is a shipped definition with `named = []`. Using the real
    // one rather than a fixture is deliberate: the bug was in how a real
    // declaration was read, and a fixture could drift away from it.
    // Tolerant of an already-initialised singleton: this binary shares one
    // `OnceLock` across every test, so whether we are first is a property of
    // test ordering, not of this test.
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let agent = OpenHumanSessionHost::from_config_for_agent(&config, "summarizer")
        .expect("summarizer is a shipped agent definition");

    let visible = agent.visible_tool_names_for_test();
    let real: Vec<&String> = visible
        .iter()
        .filter(|n| n.as_str() != crate::agent::harness::definition::NO_TOOLS_SENTINEL)
        .collect();
    assert!(
        real.is_empty(),
        "an agent declaring `named = []` must advertise nothing, got: {real:?}"
    );
}

#[tokio::test]
async fn a_zero_tool_agent_does_not_gain_the_compaction_recovery_tool() {
    // `ensure_recovery_tool_visible` joins the recovery tool to any non-empty
    // named belt, and the sentinel makes a zero-tool belt non-empty for the
    // first time. Without `is_empty_tool_scope` there, "no tools" would have
    // quietly become "one tool" — and there is nothing for it to recover,
    // because an agent with no tools produces no tool output to truncate.
    use crate::agent::session_host::types::OpenHumanSessionHost;
    use crate::inference::tokenjuice::RETRIEVE_TOOL_NAME;

    // Tolerant of an already-initialised singleton: this binary shares one
    // `OnceLock` across every test, so whether we are first is a property of
    // test ordering, not of this test.
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let agent = OpenHumanSessionHost::from_config_for_agent(&config, "trigger_triage")
        .expect("trigger_triage is a shipped agent definition");

    assert!(
        !agent
            .visible_tool_names_for_test()
            .contains(RETRIEVE_TOOL_NAME),
        "a deliberately tool-less agent must not be handed the recovery tool"
    );
}

#[test]
fn the_no_tools_sentinel_can_never_name_a_real_tool() {
    // The value is load-bearing: it works only because no registry can contain
    // it. Leading underscores are not a legal tool name for any provider's
    // function-calling schema, which is why this shape was chosen.
    use crate::agent::harness::definition::NO_TOOLS_SENTINEL;
    assert!(NO_TOOLS_SENTINEL.starts_with("__"));
    assert!(!NO_TOOLS_SENTINEL.chars().next().unwrap().is_alphanumeric());
}

#[test]
fn is_empty_tool_scope_distinguishes_the_three_states() {
    use crate::agent::harness::definition::{NO_TOOLS_SENTINEL, is_empty_tool_scope};
    use std::collections::HashSet;

    // Unset — the historical "everything" sentinel.
    assert!(is_empty_tool_scope(&HashSet::new()));
    // Deliberately empty.
    let sentinel: HashSet<String> = [NO_TOOLS_SENTINEL.to_string()].into_iter().collect();
    assert!(is_empty_tool_scope(&sentinel));
    // A real belt is neither.
    let real: HashSet<String> = ["shell".to_string()].into_iter().collect();
    assert!(!is_empty_tool_scope(&real));
    // The sentinel alongside a real tool is not an empty scope — that
    // combination should never be built, but reading it as "empty" would hide
    // a real tool from the belt rather than surface the mistake.
    let mixed: HashSet<String> = [NO_TOOLS_SENTINEL.to_string(), "shell".to_string()]
        .into_iter()
        .collect();
    assert!(!is_empty_tool_scope(&mixed));
}
