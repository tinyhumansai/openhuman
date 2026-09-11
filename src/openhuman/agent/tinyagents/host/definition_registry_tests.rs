use super::*;
use crate::openhuman::agent::registry::types::{
    AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy,
};

fn builtins() -> OpenHumanDefinitionRegistry {
    OpenHumanDefinitionRegistry::builtins_only()
}

/// A synthetic host definition built through the public
/// [`definition_from_registry_entry`] constructor, so the test never
/// hand-rolls the harness struct's ~25 fields.
fn synthetic(id: &str, tier: AgentTier, subagents: &[&str]) -> HostAgentDefinition {
    let entry = AgentRegistryEntry {
        id: id.to_string(),
        name: format!("{id} display"),
        description: format!("Use {id} for testing."),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: None,
        system_prompt: None,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::from_allowlist(
            subagents.iter().map(|s| s.to_string()).collect(),
        ),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    };
    let mut def = definition_from_registry_entry(&entry);
    def.agent_tier = tier;
    def
}

fn registry_of(defs: Vec<HostAgentDefinition>) -> OpenHumanDefinitionRegistry {
    let mut registry = AgentDefinitionRegistry::default();
    for def in defs {
        registry.insert(def);
    }
    OpenHumanDefinitionRegistry::new(Arc::new(registry))
}

// ── the absence contract ──────────────────────────────────────────────

#[tokio::test]
async fn resolve_returns_none_for_an_unknown_id_without_erroring() {
    // THE contract of this trait: OpenHuman's orchestrator TOML lists
    // subagents that a feature-gated build compiles out, and both existing
    // resolution sites tolerate that. An `Err` here would turn ordinary
    // build variance into a failed run.
    let resolved = builtins()
        .resolve("definitely_not_a_real_agent")
        .await
        .expect("an unknown id must not be an error");
    assert_eq!(resolved, None);
}

#[tokio::test]
async fn a_declared_subagent_that_does_not_resolve_is_still_authorized() {
    // The two halves of the contract together: `delegates_for` keeps an id
    // it cannot resolve (authorization and compiled-in-ness are
    // independent), and resolving that id is `Ok(None)`, not `Err`.
    let registry = registry_of(vec![synthetic(
        "lead",
        AgentTier::Chat,
        &["compiled_out_agent"],
    )]);

    let delegates = registry.delegates_for("lead").await.expect("delegates");
    assert_eq!(delegates, vec!["compiled_out_agent".to_string()]);
    assert_eq!(
        registry
            .resolve("compiled_out_agent")
            .await
            .expect("an unresolvable delegate must not error"),
        None
    );
}

#[tokio::test]
async fn delegates_for_is_empty_for_an_unknown_id() {
    let delegates = builtins()
        .delegates_for("definitely_not_a_real_agent")
        .await
        .expect("delegates");
    assert!(delegates.is_empty());
}

// ── tier checking ─────────────────────────────────────────────────────

#[tokio::test]
async fn delegates_for_drops_tier_illegal_children() {
    // `chat -> chat` and `reasoning -> reasoning` are the two forbidden
    // same-tier hops; `chat -> worker` is legal.
    let registry = registry_of(vec![
        synthetic("lead", AgentTier::Chat, &["other_chat", "a_worker"]),
        synthetic("other_chat", AgentTier::Chat, &[]),
        synthetic("a_worker", AgentTier::Worker, &[]),
    ]);

    let delegates = registry.delegates_for("lead").await.expect("delegates");
    assert_eq!(delegates, vec!["a_worker".to_string()]);
}

#[tokio::test]
async fn a_worker_parent_authorizes_nothing() {
    // `validate_tier_hierarchy` hard-fails a worker that lists any agent
    // id, so the authorized set must be empty rather than the raw list.
    let registry = registry_of(vec![
        synthetic("leaf", AgentTier::Worker, &["a_worker"]),
        synthetic("a_worker", AgentTier::Worker, &[]),
    ]);

    assert!(registry
        .delegates_for("leaf")
        .await
        .expect("delegates")
        .is_empty());
    // The *declared* list is still reported on the definition — declared
    // and authorized are different questions.
    let def = registry
        .resolve("leaf")
        .await
        .expect("resolve")
        .expect("leaf exists");
    assert_eq!(def.subagents, vec!["a_worker".to_string()]);
}

#[tokio::test]
async fn authorized_delegates_agree_with_boot_time_validation() {
    // Cross-check against the host's own boot validator over the real
    // built-in set: every id `delegates_for` returns must be one
    // `validate_tier_hierarchy` accepts (it accepts the whole builtin
    // catalogue, so nothing may be dropped there).
    let builtin_defs = crate::openhuman::agent::registry::agents::load_builtins()
        .expect("built-in TOML must parse");
    crate::openhuman::agent::registry::agents::validate_tier_hierarchy(&builtin_defs)
        .expect("built-ins satisfy the hierarchy");

    let registry = builtins();
    for def in &builtin_defs {
        let declared = declared_subagent_ids(def);
        let authorized = registry
            .delegates_for(&def.id)
            .await
            .expect("delegates never error");
        for id in &authorized {
            assert!(
                declared.contains(id),
                "{} authorized `{id}` which it never declared",
                def.id
            );
        }
        if def.agent_tier == AgentTier::Worker {
            assert!(
                authorized.is_empty(),
                "worker `{}` must authorize no delegates",
                def.id
            );
        }
    }
}

// ── projection ────────────────────────────────────────────────────────

#[tokio::test]
async fn orchestrator_projects_its_identity_fields() {
    let def = builtins()
        .resolve("orchestrator")
        .await
        .expect("resolve")
        .expect("the orchestrator is always a built-in");
    assert_eq!(def.id, "orchestrator");
    assert!(!def.name.is_empty());
    assert!(
        !def.description.is_empty(),
        "description is the delegating parent's capability summary"
    );
    assert!(
        !def.subagents.is_empty(),
        "the orchestrator declares delegates"
    );
    assert!(
        !def.tools.is_empty(),
        "the orchestrator has a named tool scope"
    );
}

#[test]
fn model_spec_maps_inherit_to_no_preference() {
    assert_eq!(model_for(&ModelSpec::Inherit), None);
    assert_eq!(
        model_for(&ModelSpec::Exact("neocortex-mk1".into())),
        Some("neocortex-mk1".to_string())
    );
    // Hints go through `ModelSpec::resolve`, which is the one place the
    // `{hint}-v1` convention lives.
    assert_eq!(
        model_for(&ModelSpec::Hint("reasoning".into())),
        Some("reasoning-v1".to_string())
    );
}

#[test]
fn skills_wildcard_entries_are_not_agent_ids() {
    use crate::openhuman::agent::harness::definition::SkillsWildcard;
    let mut def = synthetic("lead", AgentTier::Chat, &["a_worker"]);
    def.subagents.push(SubagentEntry::Skills(SkillsWildcard {
        skills: "*".to_string(),
    }));
    assert_eq!(declared_subagent_ids(&def), vec!["a_worker".to_string()]);
}

#[test]
fn denylist_supports_exact_and_prefix_forms() {
    let denied = vec!["file_write".to_string(), "storage_*".to_string()];
    assert!(disallows_tool(&denied, "file_write"));
    assert!(disallows_tool(&denied, "storage_delete_file"));
    assert!(!disallows_tool(&denied, "file_read"));
}

/// A wildcard scope with nothing denied is the one case where the crate's
/// "empty means unrestricted" marker is the faithful projection.
#[test]
fn an_undenied_wildcard_scope_projects_the_unrestricted_marker() {
    let mut def = synthetic("wide", AgentTier::Worker, &[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = Vec::new();

    assert!(
        registry_of(vec![def.clone()])
            .project(&def)
            .tools
            .is_empty(),
        "an undenied wildcard is genuinely unrestricted"
    );
}

/// A wildcard scope carrying a denylist must be materialised against the
/// registered tool list, not projected as the unrestricted marker — which
/// would hand every denied tool straight back.
///
/// Written against a synthetic definition rather than a shipped one on
/// purpose: the shipped denylists are product data and come and go (the
/// last specialist-only family left),
/// while the projection rule this pins is permanent.
#[test]
fn a_wildcard_denylist_is_materialized_against_the_registered_tools() {
    let mut def = synthetic("denier", AgentTier::Worker, &[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = vec!["secret_*".to_string()];

    let projected = registry_of(vec![def.clone()])
        .with_registered_tools(Arc::new(vec![
            "file_read".to_string(),
            "secret_read".to_string(),
        ]))
        .project(&def);

    assert_eq!(projected.tools, vec!["file_read".to_string()]);
    assert!(
        !projected.tools.iter().any(|t| t == "secret_read"),
        "a denied tool must not survive the wildcard projection"
    );
}

/// Without a registered-tool list the denylist cannot be expressed, so the
/// projection must fail closed rather than widen to everything.
#[test]
fn a_wildcard_denylist_without_registered_tools_fails_closed() {
    let mut def = synthetic("denier", AgentTier::Worker, &[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = vec!["secret_*".to_string()];

    let projected = registry_of(vec![def.clone()]).project(&def);

    assert_eq!(
        projected.tools,
        vec![PROFILE_NO_TOOLS_SENTINEL.to_string()],
        "an unexpressible denylist must not read back as 'all tools'"
    );
}

/// An agent configured with an empty allowlist wants *no* tools. The empty
/// vec would say the opposite.
#[test]
fn an_explicitly_tool_less_named_scope_projects_the_no_tools_sentinel() {
    let mut def = synthetic("toolless", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(Vec::new());
    def.extra_tools = Vec::new();

    assert_eq!(
        registry_of(vec![def.clone()]).project(&def).tools,
        vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
    );
}

/// Same requirement when the denylist is what emptied the scope.
#[test]
fn a_named_scope_emptied_by_its_denylist_projects_the_no_tools_sentinel() {
    let mut def = synthetic("denied", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["example_tool".to_string()]);
    def.extra_tools = Vec::new();
    def.disallowed_tools = vec!["example_tool".to_string()];

    assert_eq!(
        registry_of(vec![def.clone()]).project(&def).tools,
        vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
    );
}

#[test]
fn named_scope_drops_denied_tools_and_keeps_extras() {
    let mut def = synthetic("worker", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into(), "file_write".into()]);
    def.extra_tools = vec!["grep".into(), "file_read".into()];
    def.disallowed_tools = vec!["file_write".into()];

    let registry = registry_of(vec![def.clone()]);
    assert_eq!(
        registry.project(&def).tools,
        vec!["file_read".to_string(), "grep".to_string()]
    );
}

// ── profile restriction ───────────────────────────────────────────────

fn profile_allowing(tools: &[&str]) -> Arc<AgentProfile> {
    let mut profile = crate::openhuman::agent::profiles::built_in_profiles()
        .into_iter()
        .next()
        .expect("at least one built-in profile ships");
    profile.allowed_tools = Some(tools.iter().map(|t| t.to_string()).collect());
    Arc::new(profile)
}

#[test]
fn profile_allowlist_narrows_a_named_scope() {
    let mut def = synthetic("worker", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into(), "grep".into()]);

    let registry = registry_of(vec![def.clone()]).with_profile(profile_allowing(&["grep"]));
    assert_eq!(registry.project(&def).tools, vec!["grep".to_string()]);
}

#[test]
fn a_disjoint_profile_allowlist_yields_zero_tools_not_all_tools() {
    // The failure mode the host's sentinel exists to prevent: an empty list
    // reads as "unrestricted", so a disjoint intersection must stay
    // non-empty with an unregistered name.
    let mut def = synthetic("worker", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into()]);

    let registry =
        registry_of(vec![def.clone()]).with_profile(profile_allowing(&["something_else"]));
    assert_eq!(
        registry.project(&def).tools,
        vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
    );
}

#[test]
fn profile_allowlist_becomes_the_visible_set_for_a_wildcard_agent() {
    let mut def = synthetic("worker", AgentTier::Worker, &[]);
    def.tools = ToolScope::Wildcard;

    let registry = registry_of(vec![def.clone()]).with_profile(profile_allowing(&[" grep ", ""]));
    assert_eq!(registry.project(&def).tools, vec!["grep".to_string()]);
}

// ── config-backed custom agents ───────────────────────────────────────

fn config_with(entries: Vec<AgentRegistryEntry>) -> Arc<Config> {
    let mut config = Config::default();
    config.agent_registry.entries = entries;
    Arc::new(config)
}

fn custom_entry(id: &str, enabled: bool) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: "Finance Analyst".to_string(),
        description: "Handles finance questions.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled,
        model: Some("hint:reasoning".to_string()),
        system_prompt: Some("Do finance work.".to_string()),
        tool_allowlist: vec!["memory_recall".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::default(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn an_enabled_custom_config_agent_resolves_and_lists() {
    let registry =
        registry_of(Vec::new()).with_config(config_with(vec![custom_entry("finance", true)]));

    let def = registry
        .resolve("finance")
        .await
        .expect("resolve")
        .expect("an enabled custom agent is in the catalogue");
    assert_eq!(def.name, "Finance Analyst");
    assert_eq!(def.model.as_deref(), Some("reasoning-v1"));
    assert_eq!(def.tools, vec!["memory_recall".to_string()]);

    let listed = registry.list().await.expect("list");
    assert_eq!(
        listed.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        vec!["finance"]
    );
}

#[tokio::test]
async fn a_disabled_custom_config_agent_is_a_miss_not_an_error() {
    // The disabled filter lives in `find_custom_in_config`; this pins that
    // the adapter routes through it rather than reading entries directly.
    let registry =
        registry_of(Vec::new()).with_config(config_with(vec![custom_entry("finance", false)]));

    assert_eq!(registry.resolve("finance").await.expect("resolve"), None);
    assert!(registry.list().await.expect("list").is_empty());
}

#[tokio::test]
async fn a_harness_definition_shadows_a_same_id_config_entry() {
    let registry = registry_of(vec![synthetic("finance", AgentTier::Worker, &[])])
        .with_config(config_with(vec![custom_entry("finance", true)]));

    let def = registry
        .resolve("finance")
        .await
        .expect("resolve")
        .expect("present");
    assert_eq!(def.name, "finance display", "harness definition must win");
    assert_eq!(registry.list().await.expect("list").len(), 1);
}

// ── list stability ────────────────────────────────────────────────────

#[tokio::test]
async fn list_is_stable_across_calls() {
    let registry = builtins();
    let first = registry.list().await.expect("list");
    let second = registry.list().await.expect("list");
    assert!(!first.is_empty());
    assert_eq!(first, second);
}

#[tokio::test]
async fn is_usable_as_a_trait_object() {
    let registry: Box<dyn DefinitionRegistry> = Box::new(builtins());
    assert!(registry
        .resolve("orchestrator")
        .await
        .expect("resolve")
        .is_some());
}

#[tokio::test]
async fn an_empty_catalogue_misses_everything_without_erroring() {
    let registry = registry_of(Vec::new());
    assert_eq!(registry.resolve("anything").await.expect("resolve"), None);
    assert!(registry.list().await.expect("list").is_empty());
    assert!(registry
        .delegates_for("anything")
        .await
        .expect("delegates")
        .is_empty());
}
