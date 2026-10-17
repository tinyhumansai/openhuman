use super::*;

#[test]
fn default_is_the_orchestrator_under_the_agents_id() {
    let def = AgentDefinitionSpec::new()
        .into_core("alpha")
        .expect("definition");
    assert_eq!(def.id, "alpha");
    assert_eq!(def.display_name(), "alpha");
    assert!(matches!(def.system_prompt, PromptSource::Dynamic(_)));
    assert!(matches!(def.tools, ToolScope::Wildcard));
    assert_eq!(def.sandbox_mode, SandboxMode::None);
}

#[test]
fn setters_override_one_aspect_each() {
    let def = AgentDefinitionSpec::new()
        .system_prompt("Be terse.")
        .tools(ToolScopeSpec::Named(vec!["read_file".into()]))
        .disallow_tools(["shell"])
        .sandbox(SandboxModeSpec::Sandboxed)
        .max_iterations(3)
        .temperature(0.1)
        .display_name("Alpha")
        .into_core("alpha")
        .expect("definition");
    assert!(matches!(def.system_prompt, PromptSource::Inline(ref p) if p == "Be terse."));
    assert!(
        matches!(def.tools, ToolScope::Named(ref names) if names == &["read_file".to_string()])
    );
    assert!(def.disallowed_tools.iter().any(|t| t == "shell"));
    assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
    assert_eq!(def.max_iterations, 3);
    assert_eq!(def.temperature, 0.1);
    assert_eq!(def.display_name(), "Alpha");
}

#[test]
fn bare_prompt_is_verbatim_with_nothing_composed_around_it() {
    let def = AgentDefinitionSpec::new()
        .bare_prompt("Analyze.")
        .into_core("alpha")
        .expect("definition");
    assert!(matches!(def.system_prompt, PromptSource::Verbatim(ref p) if p == "Analyze."));
    assert!(
        def.extensions.omit_identity
            && def.extensions.omit_safety_preamble
            && def.extensions.omit_memory_context
    );
}

#[test]
fn system_prompt_after_bare_prompt_is_wrapped_again() {
    let def = AgentDefinitionSpec::new()
        .bare_prompt("Analyze.")
        .system_prompt("Be terse.")
        .into_core("alpha")
        .expect("definition");
    assert!(matches!(def.system_prompt, PromptSource::Inline(ref p) if p == "Be terse."));
}

#[test]
fn host_only_is_an_empty_read_only_belt_that_cannot_delegate() {
    let def = AgentDefinitionSpec::new()
        .bare_prompt("Analyze.")
        .tools(ToolScopeSpec::HostOnly)
        .sandbox(SandboxModeSpec::None)
        .into_core("alpha")
        .expect("definition");
    assert!(matches!(def.tools, ToolScope::Named(ref names) if names.is_empty()));
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(def.subagents.is_empty());
}

#[test]
fn host_only_without_a_prompt_is_refused() {
    let err = AgentDefinitionSpec::new()
        .tools(ToolScopeSpec::HostOnly)
        .into_core("alpha")
        .expect_err("the orchestrator prompt does not describe a host-only agent");
    assert!(matches!(err, AgentError::Invalid(_)));
}

#[test]
fn tool_rules_reach_the_core_definition() {
    let rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["mcp_*"]);
    let def = AgentDefinitionSpec::new()
        .tool_rules(rules.clone())
        .into_core("narrow")
        .unwrap();
    assert_eq!(def.tool_rules, Some(rules));
    assert!(AgentDefinitionSpec::new()
        .into_core("open")
        .unwrap()
        .tool_rules
        .is_none());
}

#[test]
fn template_overrides_are_independent_and_keep_restrictions() {
    let template = AgentDefinitionSpec::new()
        .system_prompt("shared")
        .temperature(0.3)
        .max_iterations(7)
        .tools(ToolScopeSpec::Named(vec![
            "read_file".into(),
            "shell".into(),
        ]));
    let first = AgentDefinitionSpec::new()
        .system_prompt("first")
        .tools(ToolScopeSpec::Named(vec!["read_file".into()]))
        .inherit(template.clone())
        .unwrap()
        .into_core("first")
        .unwrap();
    let second = AgentDefinitionSpec::new()
        .temperature(0.8)
        .inherit(template.clone())
        .unwrap()
        .into_core("second")
        .unwrap();
    assert!(matches!(first.system_prompt,PromptSource::Inline(ref p) if p=="first"));
    assert!(matches!(second.system_prompt,PromptSource::Inline(ref p) if p=="shared"));
    assert_eq!(first.temperature, 0.3);
    assert_eq!(second.temperature, 0.8);
    assert_eq!(second.max_iterations, 7);
    assert!(AgentDefinitionSpec::new()
        .tools(ToolScopeSpec::Wildcard)
        .inherit(template)
        .is_err());
}
#[test]
fn base_can_be_named_or_custom_and_unknown_names_are_typed() {
    let original = AgentDefinitionRegistry::builtins_only()
        .get("planner")
        .unwrap()
        .clone();
    let named = AgentDefinitionSpec::from_base("planner")
        .into_core("my-planner")
        .unwrap();
    assert_eq!(named.agent_tier, original.agent_tier);
    let custom = AgentDefinitionSpec::from_base(original.clone())
        .into_core("my-custom")
        .unwrap();
    assert_eq!(
        format!("{:?}", custom.tools),
        format!("{:?}", original.tools)
    );
    assert_eq!(custom.max_iterations, original.max_iterations);
    assert!(matches!(
        AgentDefinitionSpec::from_base("does-not-exist").into_core("bad"),
        Err(AgentError::UnknownDefinition(_))
    ));
}

#[test]
fn a_custom_base_does_not_allow_a_wider_tool_scope() {
    let mut base = AgentDefinitionSpec::new()
        .tools(ToolScopeSpec::Named(vec!["read_file".into()]))
        .into_core("custom-base")
        .unwrap();
    base.sandbox_mode = SandboxMode::ReadOnly;
    let def = AgentDefinitionSpec::from_base(base.clone())
        .into_core("keeps-base")
        .unwrap();
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(matches!(
        AgentDefinitionSpec::from_base(base)
            .tools(ToolScopeSpec::Wildcard)
            .into_core("widens"),
        Err(AgentError::WidensRuntime(_))
    ));
}
