use super::*;
use crate::agent_registry::types::{AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy};
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use std::fs;
use std::io::Write;
use tinyagents_definition::PromptSource;

/// Serializes the process-registry refresh tests: they mutate the process-wide
/// registry, so parallel execution would interleave their applies and asserts.
static REGISTRY_REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn named(id: &str, when: &str) -> AgentDefinition {
    let mut definition = AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .cloned()
        .expect("built-in orchestrator");
    definition.id = id.to_string();
    definition.when_to_use = when.to_string();
    definition
}

#[test]
fn with_definitions_adds_and_replaces_without_touching_the_source() {
    let base = AgentDefinitionRegistry::builtins_only();
    let before = base.len();
    let own = base.with_definitions([
        named("own-worker", "added"),
        named("orchestrator", "replaced"),
    ]);
    assert_eq!(own.len(), before + 1);
    assert_eq!(own.get("own-worker").unwrap().when_to_use, "added");
    assert_eq!(own.get("orchestrator").unwrap().when_to_use, "replaced");
    assert!(base.get("own-worker").is_none());
    assert_ne!(base.get("orchestrator").unwrap().when_to_use, "replaced");
}

#[tokio::test]
async fn current_follows_the_agent_context_and_falls_back_to_the_process() {
    let own = Arc::new(
        AgentDefinitionRegistry::builtins_only().with_definitions([named("ctx-only", "mine")]),
    );
    let parent = CoreContext::for_test(DomainSet::full(), Some(std::env::temp_dir()));
    let agent = parent.derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent("catalogue-agent")
        .definitions(Arc::clone(&own)),
    );

    let seen = CoreContext::scope(agent, async { AgentDefinitionRegistry::current() })
        .await
        .expect("the agent's registry");
    assert!(Arc::ptr_eq(&seen, &own));

    let outside = CoreContext::scope(parent, async { AgentDefinitionRegistry::current() }).await;
    assert!(
        outside.is_none_or(|registry| registry.get("ctx-only").is_none()),
        "a context without its own catalogue resolves through the process"
    );
}

#[test]
fn builtins_only_registry_holds_builtins_only() {
    assert!(AgentDefinitionRegistry::builtins_only().holds_builtins_only());
}

#[test]
fn an_extra_definition_is_not_builtins_only() {
    let extra =
        AgentDefinitionRegistry::builtins_only().with_definitions([named("own-worker", "added")]);
    assert!(!extra.holds_builtins_only());
}

#[test]
fn a_file_sourced_override_is_not_builtins_only() {
    let mut overridden = named("orchestrator", "from a workspace file");
    overridden.source = DefinitionSource::File("/ws/agents/orchestrator.toml".into());
    let registry = AgentDefinitionRegistry::builtins_only().with_definitions([overridden]);
    assert!(!registry.holds_builtins_only());
}

#[test]
fn a_builtin_tagged_override_with_other_contents_is_not_builtins_only() {
    let registry = AgentDefinitionRegistry::builtins_only()
        .with_definitions([named("orchestrator", "replaced but still tagged Builtin")]);
    assert!(!registry.holds_builtins_only());
}

fn write_toml(path: &Path, contents: &str) {
    let mut f = fs::File::create(path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

fn fresh_workspace() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Definitions the registry layers over the built-ins from a workspace.
fn user_defs(workspace: &Path) -> Vec<AgentDefinition> {
    AgentDefinitionRegistry::load(workspace)
        .unwrap()
        .list()
        .into_iter()
        .filter(|d| matches!(d.source, DefinitionSource::File(_)))
        .cloned()
        .collect()
}

// NOTE: TOML parsing is positional. Top-level scalars MUST come
// before any `[table]` header — once a header opens, every line
// below it lives inside that table.
const NOTION_TOML: &str = r#"
id = "notion_specialist"
when_to_use = "Delegate Notion queries to a focused specialist."
display_name = "Notion Specialist"
temperature = 0.4
skill_filter = "notion"
max_iterations = 5

[system_prompt]
inline = "You are the Notion specialist. Use only Notion tools."

[model]
hint = "agentic"
"#;

#[test]
fn loads_single_definition_from_workspace() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

    let defs = user_defs(ws.path());
    assert_eq!(defs.len(), 1);
    let def = &defs[0];
    assert_eq!(def.id, "notion_specialist");
    assert_eq!(def.skill_filter.as_deref(), Some("notion"));
    assert_eq!(def.max_iterations, 5);
    assert!(matches!(def.source, DefinitionSource::File(_)));
}

#[test]
fn empty_when_no_agents_dir() {
    let ws = fresh_workspace();
    let defs = user_defs(ws.path());
    assert!(defs.is_empty());
}

#[test]
fn ignores_non_toml_files() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    write_toml(&agents_dir.join("readme.md"), "not a definition");
    write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

    let defs = user_defs(ws.path());
    assert_eq!(defs.len(), 1);
}

#[test]
fn skips_malformed_files_without_aborting() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    write_toml(&agents_dir.join("broken.toml"), "id = \"broken\"  [oops");
    write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

    let defs = user_defs(ws.path());
    // The broken file is skipped; the valid one still loads.
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].id, "notion_specialist");
}

#[test]
fn registry_load_merges_builtins_and_custom() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

    let reg = AgentDefinitionRegistry::load(ws.path()).unwrap();
    // The built-in set is allowed to grow over time (new archetypes,
    // additional synthetic definitions), so assert presence of the
    // specific ids we care about rather than a fixed total count.
    assert!(
        reg.len() > 1,
        "expected at least one built-in plus the custom definition"
    );
    assert!(reg.get("notion_specialist").is_some());
    assert!(reg.get("summarizer").is_some());
}

#[test]
fn rejects_definition_with_missing_system_prompt() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    // No `[system_prompt]` table — serde falls back to the empty
    // inline placeholder, which the loader must reject.
    write_toml(
        &agents_dir.join("broken.toml"),
        r#"
id = "broken"
when_to_use = "should be rejected"
"#,
    );
    let defs = user_defs(ws.path());
    assert!(
        defs.is_empty(),
        "expected loader to reject definition without system_prompt"
    );
}

#[test]
fn custom_definition_overrides_same_id_builtin() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    // Override the built-in `summarizer` with a custom one.
    write_toml(
        &agents_dir.join("summarizer.toml"),
        r#"
id = "summarizer"
when_to_use = "CUSTOM OVERRIDE"

[system_prompt]
inline = "custom prompt"
"#,
    );

    // Load a baseline registry (no custom overrides) to get the
    // built-in count dynamically — avoids coupling to a hardcoded number.
    let baseline =
        AgentDefinitionRegistry::load(&tempfile::TempDir::new().unwrap().path().join("empty"))
            .unwrap();
    let expected_count = baseline.len();

    let reg = AgentDefinitionRegistry::load(ws.path()).unwrap();
    // Same id replaced the built-in `summarizer` in place, so the
    // registry size doesn't grow when the custom TOML collides.
    assert_eq!(reg.len(), expected_count);
    let def = reg.get("summarizer").unwrap();
    assert_eq!(def.when_to_use, "CUSTOM OVERRIDE");
    assert!(matches!(def.source, DefinitionSource::File(_)));
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
    let defs = crate::agent_registry::agents::load_builtins()
        .expect("built-in agent TOML must always parse");

    let expected: &[(&str, usize)] = &[
        // Extended policy (or high `max_iterations`) -> effective cap raised.
        ("orchestrator", 200),
        ("task_manager_agent", 50),
        // Gated with `flows` (#4797) — absent from a slim build.
        #[cfg(feature = "flows")]
        ("flow_discovery", 50),
        #[cfg(feature = "flows")]
        ("workflow_builder", 50),
        // Strict policy, declared `max_iterations` below the old global
        // default (10) -> effective cap lowered.
        ("image_agent", 8),
        ("morning_briefing", 8),
        ("summarizer", 1),
        ("trigger_reactor", 6),
        ("trigger_triage", 2),
        ("video_agent", 8),
        ("vision_agent", 6),
        // Compiled out with the `documents` gate — see `openhuman::agent_registry::agents::loader::builtin_enabled`.
        #[cfg(feature = "documents")]
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

/// A definition written before `omit_skills_catalog` was removed (#5699) must
/// still load. `AgentDefinition` does not set `#[serde(deny_unknown_fields)]`,
/// so the retired key is ignored rather than rejected — that is what makes the
/// removal a non-breaking change for custom TOML definitions already on disk.
/// This pins it, so a future `deny_unknown_fields` cannot silently break them.
#[test]
fn a_definition_carrying_the_retired_skills_catalog_key_still_loads() {
    let toml_src = r#"
id = "legacy_agent"
display_name = "Legacy"
when_to_use = "A definition written before the flag was retired."
omit_identity = true
omit_skills_catalog = true
omit_safety_preamble = true

[tools]
named = []
"#;
    let def: AgentDefinition =
        toml::from_str(toml_src).expect("a definition carrying the retired key must still parse");
    assert_eq!(def.id, "legacy_agent");
    // The neighbouring flags still land, so the retired key is being skipped
    // rather than derailing the rest of the parse.
    assert!(def.extensions.omit_identity);
    assert!(def.extensions.omit_safety_preamble);
}

#[test]
fn host_fields_default_to_the_lean_prompt_when_the_toml_omits_them() {
    let def: AgentDefinition = toml::from_str(
        r#"
id = "lean"
when_to_use = "defaults"
"#,
    )
    .unwrap();
    assert!(def.extensions.omit_identity);
    assert!(def.extensions.omit_memory_context);
    assert!(def.extensions.omit_safety_preamble);
    assert!(!def.extensions.searches_connected_mcp);
    assert!(!def.extensions.refresh_prompt_on_resume);
    assert!(def.extensions.prompt_builder.is_none());
}

#[test]
fn host_fields_parse_from_the_flattened_top_level_of_the_toml() {
    let def: AgentDefinition = toml::from_str(
        r#"
id = "opted_in"
when_to_use = "host flags"
omit_identity = false
searches_connected_mcp = true
refresh_prompt_on_resume = true
tokenjuice_compression = "light"
"#,
    )
    .unwrap();
    assert!(!def.extensions.omit_identity);
    assert!(def.extensions.searches_connected_mcp);
    assert!(def.extensions.refresh_prompt_on_resume);
    assert_eq!(
        def.extensions.tokenjuice_compression,
        tinyjuice_bus::types::AgentTokenjuiceCompression::Light
    );
}

#[test]
fn builtin_dynamic_prompts_carry_their_builder_beside_an_inert_marker() {
    let defs = crate::agent_registry::agents::load_builtins().unwrap();
    assert!(!defs.is_empty());
    for def in &defs {
        assert!(
            matches!(def.system_prompt, PromptSource::Dynamic(())),
            "built-in `{}` uses a dynamic prompt",
            def.id
        );
        assert!(
            def.extensions.prompt_builder.is_some(),
            "built-in `{}` must keep its executable prompt builder",
            def.id
        );
        let json = serde_json::to_value(def).unwrap();
        assert_eq!(
            json["system_prompt"],
            serde_json::json!({ "dynamic": null })
        );
        assert!(
            json.get("prompt_builder").is_none(),
            "the builder is runtime-only and never serialised"
        );
    }
}

#[test]
fn coding_hint_resolves_auto_tokenjuice_to_light() {
    use tinyjuice_bus::types::AgentTokenjuiceCompression as Profile;
    let ext = AgentExt::default();
    assert_eq!(
        ext.tokenjuice_for(&ModelSpec::Hint(" Coding ".into())),
        Profile::Light
    );
    assert_eq!(ext.tokenjuice_for(&ModelSpec::Inherit), Profile::Full);
}

/// Pins tinyagents#441: existing definitions on disk spell an unrestricted
/// scope as `[tools] wildcard = {}`. Requires `vendor/tinyagents` at or after
/// tinyagents `2e706e81`.
#[test]
fn a_legacy_wildcard_table_tool_scope_still_loads_from_a_user_definition() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    write_toml(
        &agents_dir.join("legacy.toml"),
        r#"
id = "legacy_wildcard"
when_to_use = "legacy tools table"

[system_prompt]
inline = "legacy"

[tools]
wildcard = {}
"#,
    );
    let defs = user_defs(ws.path());
    assert_eq!(
        defs.len(),
        1,
        "the legacy wildcard spelling must not be skipped"
    );
    assert!(matches!(
        defs[0].tools,
        tinyagents_definition::ToolScope::Wildcard
    ));
}
fn registry_entry_with_allowlist(id: &str, allowlist: Vec<String>) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        source: AgentRegistrySource::Default,
        enabled: true,
        model: None,
        system_prompt: None,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        tool_rules: None,
        subagents: AgentSubagentPolicy::from_allowlist(allowlist),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }
}

fn custom_registry_entry(id: &str, allowlist: Vec<String>) -> AgentRegistryEntry {
    let mut entry = registry_entry_with_allowlist(id, allowlist);
    entry.source = AgentRegistrySource::Custom;
    entry
}

#[test]
fn entry_overlay_replaces_the_harness_subagents_allowlist() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let compiled: Vec<String> = registry
        .get("orchestrator")
        .expect("built-in orchestrator")
        .allowed_subagent_ids();
    assert!(
        !compiled.is_empty(),
        "the test needs a non-empty compiled allowlist to prove the override wins"
    );

    let entry = registry_entry_with_allowlist("orchestrator", vec!["researcher".to_string()]);
    let overlaid = registry
        .with_entry_overlay(&entry)
        .expect("orchestrator is a harness definition");
    assert_eq!(
        overlaid
            .get("orchestrator")
            .expect("orchestrator")
            .allowed_subagent_ids(),
        vec!["researcher".to_string()],
        "the registry entry's allowlist must win over the compiled one (#6934)"
    );
    // The receiver is untouched: the overlay returns a copy.
    assert_eq!(
        registry
            .get("orchestrator")
            .expect("orchestrator")
            .allowed_subagent_ids(),
        compiled
    );
}

#[test]
fn entry_overlay_skips_ids_the_harness_does_not_know() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let entry = registry_entry_with_allowlist("researcher", vec!["other".to_string()]);
    assert!(
        registry.with_entry_overlay(&entry).is_none(),
        "custom agents resolve through find_custom_in_config at session build, \
         not through the harness registry"
    );
}

#[test]
fn apply_registry_entries_refreshes_the_process_registry() {
    let _refresh_lock = REGISTRY_REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    /// Restores the process registry on drop, so parallel tests are
    /// unaffected even when an assertion below panics.
    struct RestoreGlobal(Option<std::sync::Arc<AgentDefinitionRegistry>>);
    impl Drop for RestoreGlobal {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                AgentDefinitionRegistry::set_global(previous);
            }
        }
    }

    AgentDefinitionRegistry::init_global_builtins().expect("init");
    let _restore = RestoreGlobal(Some(
        AgentDefinitionRegistry::global().expect("initialised"),
    ));

    let entry = registry_entry_with_allowlist("orchestrator", vec!["researcher".to_string()]);
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&entry));

    let after = AgentDefinitionRegistry::global().expect("still initialised");
    assert_eq!(
        after
            .get("orchestrator")
            .expect("orchestrator")
            .allowed_subagent_ids(),
        vec!["researcher".to_string()],
        "update_agent must reach the process registry without a restart (#6934)"
    );
}

#[test]
fn apply_registry_entries_empty_refresh_restores_pristine_policy() {
    let _refresh_lock = REGISTRY_REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    /// Restores the process registry on drop, so parallel tests are
    /// unaffected even when an assertion below panics.
    struct RestoreGlobal(Option<std::sync::Arc<AgentDefinitionRegistry>>);
    impl Drop for RestoreGlobal {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                AgentDefinitionRegistry::set_global(previous);
            }
        }
    }

    AgentDefinitionRegistry::init_global_builtins().expect("init");
    let _restore = RestoreGlobal(Some(
        AgentDefinitionRegistry::global().expect("initialised"),
    ));

    let compiled: Vec<String> = AgentDefinitionRegistry::global()
        .expect("initialised")
        .get("orchestrator")
        .expect("built-in orchestrator")
        .allowed_subagent_ids();
    assert!(
        !compiled.is_empty() && compiled != vec!["researcher".to_string()],
        "the test needs a compiled allowlist that differs from the override"
    );

    // Override, then refresh with an empty list: the harness policy must win
    // again instead of the stale override lingering.
    let entry = registry_entry_with_allowlist("orchestrator", vec!["researcher".to_string()]);
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&entry));
    AgentDefinitionRegistry::apply_registry_entries(&[]);

    assert_eq!(
        AgentDefinitionRegistry::global()
            .expect("still initialised")
            .get("orchestrator")
            .expect("orchestrator")
            .allowed_subagent_ids(),
        compiled,
        "an empty refresh must restore the pristine harness policy"
    );
}

#[test]
fn apply_registry_entries_removed_entry_does_not_linger() {
    let _refresh_lock = REGISTRY_REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    /// Restores the process registry on drop, so parallel tests are
    /// unaffected even when an assertion below panics.
    struct RestoreGlobal(Option<std::sync::Arc<AgentDefinitionRegistry>>);
    impl Drop for RestoreGlobal {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                AgentDefinitionRegistry::set_global(previous);
            }
        }
    }

    AgentDefinitionRegistry::init_global_builtins().expect("init");
    let _restore = RestoreGlobal(Some(
        AgentDefinitionRegistry::global().expect("initialised"),
    ));

    let compiled: Vec<String> = AgentDefinitionRegistry::global()
        .expect("initialised")
        .get("orchestrator")
        .expect("built-in orchestrator")
        .allowed_subagent_ids();
    assert!(
        !compiled.is_empty() && compiled != vec!["researcher".to_string()],
        "the test needs a compiled allowlist that differs from the override"
    );

    // The second refresh drops the orchestrator override entirely: the
    // orchestrator policy must fall back to the pristine harness one.
    let first = registry_entry_with_allowlist("orchestrator", vec!["researcher".to_string()]);
    let second = registry_entry_with_allowlist("summarizer", vec!["orchestrator".to_string()]);
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&first));
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&second));

    let after = AgentDefinitionRegistry::global().expect("still initialised");
    assert_eq!(
        after
            .get("orchestrator")
            .expect("orchestrator")
            .allowed_subagent_ids(),
        compiled,
        "a removed entry must not linger in the process registry"
    );
    assert_eq!(
        after
            .get("summarizer")
            .expect("summarizer")
            .allowed_subagent_ids(),
        vec!["orchestrator".to_string()],
        "the current entry list still applies"
    );
}

#[test]
fn apply_registry_entries_registers_enabled_custom_agents() {
    let _refresh_lock = REGISTRY_REFRESH_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    /// Restores the process registry on drop, so parallel tests are
    /// unaffected even when an assertion below panics.
    struct RestoreGlobal(Option<std::sync::Arc<AgentDefinitionRegistry>>);
    impl Drop for RestoreGlobal {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                AgentDefinitionRegistry::set_global(previous);
            }
        }
    }

    AgentDefinitionRegistry::init_global_builtins().expect("init");
    let _restore = RestoreGlobal(Some(
        AgentDefinitionRegistry::global().expect("initialised"),
    ));

    // A custom id the parent allowlist permits must resolve to a runnable
    // definition, not be rejected as unknown at spawn.
    let custom = custom_registry_entry("finance_analyst", vec!["researcher".to_string()]);
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&custom));

    let after = AgentDefinitionRegistry::global().expect("still initialised");
    let def = after
        .get("finance_analyst")
        .expect("enabled custom agent is registered");
    assert_eq!(def.id, "finance_analyst");
    assert_eq!(
        def.allowed_subagent_ids(),
        vec!["researcher".to_string()],
        "the custom entry's allowlist carries over"
    );

    // A disabled custom entry stays unresolvable, mirroring
    // `find_custom_in_config`.
    let mut disabled = custom_registry_entry("disabled_analyst", vec!["researcher".to_string()]);
    disabled.enabled = false;
    AgentDefinitionRegistry::apply_registry_entries(std::slice::from_ref(&disabled));
    assert!(
        AgentDefinitionRegistry::global()
            .expect("still initialised")
            .get("disabled_analyst")
            .is_none(),
        "disabled custom entries must not be registered"
    );
}
