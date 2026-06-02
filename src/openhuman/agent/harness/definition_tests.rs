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
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
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
        super::super::tool_loop::EXTENDED_MAX_TOOL_ITERATIONS
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
        super::super::tool_loop::EXTENDED_MAX_TOOL_ITERATIONS
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
