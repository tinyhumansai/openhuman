//! Tests for the builder module — dedup_visible_tool_specs and related logic.

use super::{
    dedup_visible_tool_specs, ensure_recovery_tool_visible, should_synthesize_delegation_tools,
    visible_tool_specs_for_policy,
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

#[path = "builder_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "builder_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "builder_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "builder_tests_part_04_tests.rs"]
mod part_04_tests;

// ── use_skill's advertised spec is scoped to the session ────────────────────

use crate::openhuman::tools::agent_policy::{
    TaskProfile, TaskRiskLevel, ToolPolicyAction, ToolPolicyDecision, ToolPolicySession,
};
use crate::openhuman::tools::traits::PermissionLevel;

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
    let mut tools: Vec<Box<dyn crate::openhuman::tools::traits::Tool>> = Vec::new();
    crate::openhuman::tools::toolpacks::append_pack_tools(&mut tools);
    let tool = tools
        .iter()
        .find(|t| t.name() == crate::openhuman::tools::toolpacks::USE_SKILL)
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
    let session = session_allowing(&[
        "run_workflow",
        crate::openhuman::tools::toolpacks::USE_SKILL,
    ]);

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    let load = out
        .iter()
        .find(|s| s.name == crate::openhuman::tools::toolpacks::USE_SKILL)
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
    let session = session_allowing(&[crate::openhuman::tools::toolpacks::USE_SKILL]);

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
    use crate::openhuman::tools::agent_policy::ToolPolicyEngine;
    use crate::openhuman::tools::toolpacks::{append_pack_tools, strip_packed_from_visible};

    struct Fake(&'static str);
    #[async_trait::async_trait]
    impl crate::openhuman::tools::traits::Tool for Fake {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::tools::traits::ToolResult> {
            Ok(crate::openhuman::tools::traits::ToolResult::success("ok"))
        }
    }

    let mut tools: Vec<Box<dyn crate::openhuman::tools::traits::Tool>> =
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
        names.contains(&crate::openhuman::tools::toolpacks::USE_SKILL),
        "use_skill must survive — the pack it opens is reachable: {names:?}"
    );
    let load = out
        .iter()
        .find(|s| s.name == crate::openhuman::tools::toolpacks::USE_SKILL)
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
            .contains(crate::openhuman::tools::toolpacks::USE_SKILL),
        "precondition: use_skill itself is not in the allowlist"
    );

    let specs: Vec<std::sync::Arc<ToolSpec>> =
        vec![std::sync::Arc::new(use_skill_spec_from_registry())];
    let visible: std::collections::HashSet<String> = specs.iter().map(|s| s.name.clone()).collect();

    let out = visible_tool_specs_for_policy(&specs, &visible, &session);
    let load = out
        .iter()
        .find(|s| s.name == crate::openhuman::tools::toolpacks::USE_SKILL)
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
