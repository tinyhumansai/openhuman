use super::*;
use tinyagents_definition::{ModelSpec, SubagentEntry};

/// Extract the materialized allowlist, failing if projection widens to wildcard.
trait NamedToolScope {
    fn named_tools(&self) -> Vec<String>;
}

impl NamedToolScope for tinyagents_definition::ToolScope {
    fn named_tools(&self) -> Vec<String> {
        match self {
            Self::Named(names) => names.clone(),
            Self::Wildcard => panic!("host catalogue must materialize tool scope"),
        }
    }
}
use crate::agent_registry::types::{AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy};

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
        tool_rules: None,
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

#[test]
fn projection_preserves_the_validated_tier_and_model_routing_role() {
    for tier in [AgentTier::Chat, AgentTier::Reasoning, AgentTier::Worker] {
        let def = synthetic("tiered", tier, &[]);
        let projected = registry_of(vec![def.clone()]).project(&def);
        assert_eq!(projected.role.as_deref(), Some(tier.as_str()));
        assert_eq!(projected.agent_tier.as_str(), tier.as_str());
    }
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
    // The upstream definition set rejects worker-to-agent delegation, and the
    // runtime registry still must not authorize any worker children.
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
    assert_eq!(def.allowed_subagent_ids(), vec!["a_worker".to_string()]);
}

#[tokio::test]
async fn authorized_delegates_are_declared_by_builtins() {
    // The upstream DefinitionSet validates the built-in hierarchy during
    // loading. This test checks the host registry's authorization view.
    let builtin_defs =
        crate::agent_registry::agents::load_builtins().expect("built-in TOML must parse");

    let registry = builtins();
    for def in &builtin_defs {
        let declared = def.allowed_subagent_ids();
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
    assert!(!def.display_name().is_empty());
    assert!(
        !def.when_to_use.is_empty(),
        "description is the delegating parent's capability summary"
    );
    assert!(
        !def.subagents.is_empty(),
        "the orchestrator declares delegates"
    );
    assert!(
        !def.tools.named_tools().is_empty(),
        "the orchestrator has a named tool scope"
    );
}

#[test]
fn projected_model_preserves_inherit_exact_and_hint_routing() {
    use tinyagents_definition::ModelSpec as HarnessModel;
    for (model, expected) in [
        (ModelSpec::Inherit, HarnessModel::Inherit),
        (
            ModelSpec::Exact("neocortex-mk1".into()),
            HarnessModel::Exact("neocortex-mk1".into()),
        ),
        (
            ModelSpec::Hint("reasoning".into()),
            HarnessModel::Hint("reasoning".into()),
        ),
    ] {
        let mut def = synthetic("routed", AgentTier::Worker, &[]);
        def.model = model;
        assert_eq!(registry_of(vec![def.clone()]).project(&def).model, expected);
    }
}

#[test]
fn skills_wildcard_entries_are_not_agent_ids() {
    use tinyagents_definition::SkillsWildcard;
    let mut def = synthetic("lead", AgentTier::Chat, &["a_worker"]);
    def.subagents.push(SubagentEntry::Skills(SkillsWildcard {
        skills: "*".to_string(),
    }));
    assert_eq!(def.allowed_subagent_ids(), vec!["a_worker".to_string()]);
}

#[test]
fn denylist_supports_exact_and_prefix_forms() {
    let denied = vec!["file_write".to_string(), "storage_*".to_string()];
    assert!(crate::tools::rules::glob_list_matches(
        &denied,
        "file_write"
    ));
    assert!(crate::tools::rules::glob_list_matches(
        &denied,
        "storage_delete_file"
    ));
    assert!(!crate::tools::rules::glob_list_matches(
        &denied,
        "file_read"
    ));
}

/// A wildcard scope materializes the session's registered tool surface.
#[test]
fn an_undenied_wildcard_scope_projects_registered_tools() {
    let mut def = synthetic("wide", AgentTier::Worker, &[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = Vec::new();

    assert_eq!(
        registry_of(vec![def.clone()])
            .with_registered_tools(Arc::new(vec!["file_read".to_string()]))
            .project(&def)
            .tools
            .named_tools(),
        vec!["file_read".to_string()]
    );
}

#[test]
fn named_tool_search_grants_deferred_tools_to_hosted_run() {
    let mut with_search = synthetic("searcher", AgentTier::Chat, &[]);
    with_search.tools = ToolScope::Named(vec!["tool_search".to_owned()]);
    let registry = registry_of(vec![with_search.clone()]).with_deferred_tools(Arc::new(vec![
        "desktop_list_apps".to_owned(),
        "desktop_goal".to_owned(),
    ]));
    let tools = registry.project(&with_search).tools.named_tools();
    assert!(tools.iter().any(|name| name == "tool_search"));
    assert!(tools.iter().any(|name| name == "desktop_list_apps"));
    assert!(tools.iter().any(|name| name == "desktop_goal"));

    let mut without_search = synthetic("no-search", AgentTier::Chat, &[]);
    without_search.tools = ToolScope::Named(vec!["file_read".to_owned()]);
    let tools = registry.project(&without_search).tools.named_tools();
    assert!(!tools.iter().any(|name| name.starts_with("desktop_")));
}

#[test]
fn named_discovery_scope_authorizes_only_registered_deferred_tools() {
    let mut def = synthetic("searcher", AgentTier::Chat, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into(), "tool_search".into()]);
    def.disallowed_tools = vec!["blocked_*".into()];
    let projected = registry_of(vec![def.clone()])
        .with_deferred_tools(Arc::new(vec![
            "browser_open".into(),
            "browser".into(),
            "blocked_secret".into(),
        ]))
        .project(&def);
    assert!(projected
        .tools
        .named_tools()
        .contains(&"browser_open".to_string()));
    assert!(projected
        .tools
        .named_tools()
        .contains(&"browser".to_string()));
    assert!(!projected
        .tools
        .named_tools()
        .contains(&"blocked_secret".to_string()));

    def.tools = ToolScope::Named(vec!["file_read".into()]);
    let without_discovery = registry_of(vec![def.clone()])
        .with_deferred_tools(Arc::new(vec!["browser_open".into()]))
        .project(&def);
    assert!(!without_discovery
        .tools
        .named_tools()
        .contains(&"browser_open".to_string()));
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

    assert_eq!(projected.tools.named_tools(), vec!["file_read".to_string()]);
    assert!(
        !projected
            .tools
            .named_tools()
            .iter()
            .any(|t| t == "secret_read"),
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
        projected.tools.named_tools(),
        vec![NO_TOOLS_SENTINEL.to_string()],
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
        registry_of(vec![def.clone()])
            .project(&def)
            .tools
            .named_tools(),
        vec![NO_TOOLS_SENTINEL.to_string()]
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
        registry_of(vec![def.clone()])
            .project(&def)
            .tools
            .named_tools(),
        vec![NO_TOOLS_SENTINEL.to_string()]
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
        registry.project(&def).tools.named_tools(),
        vec!["file_read".to_string(), "grep".to_string()]
    );
}

#[test]
fn the_projection_carries_the_definition_rule_layer() {
    let mut def = synthetic("worker", AgentTier::Worker, &[]);
    def.disallowed_tools = vec!["web_*".into()];
    let projected = registry_of(vec![def.clone()]).project(&def);
    let layer = projected.tool_rules.expect("a rule layer");
    assert_eq!(layer.name.as_deref(), Some("agent:worker"));
    let set = tinytools::ToolRuleSet::single(layer);
    let context = tinytools::RuleContext::new();
    assert!(!set.visible(
        &tinytools::ToolSubject::named("web_fetch"),
        &context,
        tinytools::Surface::Search
    ));

    let mut open = synthetic("open", AgentTier::Worker, &[]);
    open.disallowed_tools.clear();
    assert!(registry_of(vec![open.clone()])
        .project(&open)
        .tool_rules
        .is_none());
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
        tool_rules: None,
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
    assert_eq!(def.display_name(), "Finance Analyst");
    assert_eq!(def.model.hint().as_deref(), Some("reasoning"));
    assert_eq!(def.model.pin(), None);
    assert_eq!(def.tools.named_tools(), vec!["memory_recall".to_string()]);

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
    assert_eq!(
        def.display_name(),
        "finance display",
        "harness definition must win"
    );
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

/// The defect behind #6404 / #6392 / #6393, at the layer that caused it.
///
/// A library host builds its agent from a definition it owns and never
/// registers: `AgentSpec::into_core` re-stamps the built-in orchestrator under
/// the caller's id, so `harness` / `alpha` / `beta` reach hosted resolution as
/// ids no registry holds. Before the session-definition seam this lookup
/// missed, `prepare_agent_turn` raised `TinyAgentsError::Validation`, and the
/// harness's `hosted_error` sanitized that into "hosted agent invocation was
/// rejected by policy" — reported with zero provider calls, because the miss
/// happens during turn preparation, before the loop runs.
#[tokio::test]
async fn a_caller_supplied_definition_resolves_under_its_own_unregistered_id() {
    // Exactly the shape the embed harness produces: an id no registry knows.
    let caller = synthetic("harness", AgentTier::Worker, &[]);
    let registry = registry_of(vec![synthetic("orchestrator", AgentTier::Chat, &[])]);

    // Without the session definition the id is simply absent — this is the
    // miss that became "rejected by policy".
    assert!(
        registry.resolve("harness").await.unwrap().is_none(),
        "precondition: an unregistered caller id must not resolve from the registry alone"
    );

    let with_session = registry_of(vec![synthetic("orchestrator", AgentTier::Chat, &[])])
        .with_session_definition(Arc::new(caller));
    let resolved = with_session
        .resolve("harness")
        .await
        .unwrap()
        .expect("the session's own definition must resolve under its own id");
    assert_eq!(resolved.id, "harness");
}

/// The session's own definition outranks a same-id registry entry.
///
/// This is the precedence `OpenHumanSessionHost::resolved_definition` already
/// documents and `resolved_definition_prefers_the_sessions_own_over_a_same_id_registry_entry`
/// pins for the session's own reads; hosted resolution must not disagree with
/// it, or a library host reusing a built-in id would silently run the
/// built-in's definition instead of its own.
#[tokio::test]
async fn the_sessions_own_definition_outranks_a_same_id_registry_entry() {
    let mut caller = synthetic("orchestrator", AgentTier::Worker, &[]);
    caller.display_name = Some("the caller's own".to_string());

    let registry = registry_of(vec![synthetic("orchestrator", AgentTier::Chat, &[])])
        .with_session_definition(Arc::new(caller));

    let resolved = registry
        .resolve("orchestrator")
        .await
        .unwrap()
        .expect("orchestrator resolves");
    // `project` maps `name` from `display_name()`, which only the caller's copy
    // sets — had the registry entry won, this would be its own display name.
    assert_eq!(
        resolved.display_name(),
        "the caller's own",
        "the session's own definition must win over the same-id registry entry"
    );
    // And the tier travels with it: the registry entry is Chat-tier (what the
    // real `orchestrator` is), the caller's copy is a Worker, so a stale
    // registry hit would show up here too.
    assert_eq!(
        resolved.role.as_deref(),
        Some(AgentTier::Worker.to_string()).as_deref(),
        "the winning definition's tier must be the caller's"
    );
}

/// A session definition must not answer for an id that is not its own.
#[tokio::test]
async fn a_session_definition_does_not_shadow_other_ids() {
    let registry = registry_of(vec![synthetic("orchestrator", AgentTier::Chat, &[])])
        .with_session_definition(Arc::new(synthetic("harness", AgentTier::Worker, &[])));

    assert_eq!(
        registry
            .resolve("orchestrator")
            .await
            .unwrap()
            .map(|def| def.id),
        Some("orchestrator".to_string()),
        "a session definition must not capture lookups for other ids"
    );
    assert!(
        registry.resolve("no-such-agent").await.unwrap().is_none(),
        "a session definition must not answer for an unrelated missing id"
    );
}

fn handle_mode_config() -> Arc<crate::config::Config> {
    let mut config = crate::config::Config::default();
    config.context.compaction_enabled = true;
    config.tokenjuice.router_enabled = true;
    config.tokenjuice.ccr_enabled = true;
    config.tokenjuice.repl_handle_enabled = true;
    Arc::new(config)
}

/// A curated belt must still be allowed to call the tools a compacted result
/// points at. The session advertises them; the harness allowlist built here is
/// what decides whether a call to them dispatches or is answered as unknown.
#[test]
fn named_scope_admits_the_tinyjuice_tools_a_handle_names() {
    let mut def = synthetic("curated", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into()]);
    let registered = Arc::new(vec![
        "file_read".to_string(),
        "juice_retrieve".to_string(),
        "juice_find".to_string(),
        "juice_extract".to_string(),
        "juice_summarize".to_string(),
    ]);
    let tools = registry_of(vec![def.clone()])
        .with_config(handle_mode_config())
        .with_registered_tools(registered)
        .project(&def)
        .tools
        .named_tools();
    for name in [
        "juice_retrieve",
        "juice_find",
        "juice_extract",
        "juice_summarize",
    ] {
        assert!(
            tools.contains(&name.to_string()),
            "{name} missing: {tools:?}"
        );
    }
}

#[test]
fn named_scope_skips_tinyjuice_tools_that_are_off_or_unregistered() {
    let mut def = synthetic("curated", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(vec!["file_read".into()]);

    // Handle mode off: nothing names the REPL tools, so they stay out.
    let mut off = crate::config::Config::default();
    off.context.compaction_enabled = false;
    off.tokenjuice.repl_handle_enabled = false;
    let tools = registry_of(vec![def.clone()])
        .with_config(Arc::new(off))
        .with_registered_tools(Arc::new(vec!["file_read".into(), "juice_find".into()]))
        .project(&def)
        .tools
        .named_tools();
    assert!(!tools.contains(&"juice_find".to_string()), "{tools:?}");

    // Active but not registered: an allowlist entry for a tool that does not
    // exist would only widen the surface.
    let tools = registry_of(vec![def.clone()])
        .with_config(handle_mode_config())
        .with_registered_tools(Arc::new(vec!["file_read".into()]))
        .project(&def)
        .tools
        .named_tools();
    assert!(!tools.contains(&"juice_find".to_string()), "{tools:?}");
}

#[test]
fn zero_tool_scope_stays_zero_tool_with_tinyjuice_active() {
    let mut def = synthetic("silent", AgentTier::Worker, &[]);
    def.tools = ToolScope::Named(Vec::new());
    let tools = registry_of(vec![def.clone()])
        .with_config(handle_mode_config())
        .with_registered_tools(Arc::new(vec!["juice_find".into()]))
        .project(&def)
        .tools
        .named_tools();
    assert!(!tools.contains(&"juice_find".to_string()), "{tools:?}");
}
