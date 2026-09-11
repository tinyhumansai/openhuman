use super::*;

#[test]
fn default_agents_include_core_personas() {
    let agents = default_agents();
    let ids: Vec<&str> = agents.iter().map(|agent| agent.id.as_str()).collect();
    assert!(ids.contains(&"orchestrator"));
    assert!(ids.contains(&"researcher"));
    assert!(ids.contains(&"code_executor"));
    assert!(agents
        .iter()
        .all(|agent| agent.source == AgentRegistrySource::Default));
}

fn custom_entry(id: &str) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: id.to_string(),
        name: "Finance Analyst".to_string(),
        description: "Reviews spend and drafts finance summaries.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: Some("hint:reasoning".to_string()),
        system_prompt: Some("You are a meticulous finance analyst.".to_string()),
        tool_allowlist: vec!["memory_search".to_string(), "web_search".to_string()],
        tool_denylist: vec!["file_write".to_string()],
        subagents: AgentSubagentPolicy::from_allowlist(vec!["researcher".to_string()]),
        tags: vec!["finance".to_string()],
        metadata: Value::Null,
    }
}

#[test]
fn definition_from_registry_entry_preserves_tools_model_denylist_subagents() {
    let entry = custom_entry("finance_analyst");
    let def = definition_from_registry_entry(&entry);

    assert_eq!(def.id, "finance_analyst");
    assert_eq!(def.when_to_use, entry.description);
    assert_eq!(def.display_name(), "Finance Analyst");
    assert!(matches!(
        def.model,
        ModelSpec::Hint(ref hint) if hint == "reasoning"
    ));
    assert!(matches!(
        def.tools,
        ToolScope::Named(ref names)
            if names == &vec!["memory_search".to_string(), "web_search".to_string()]
    ));
    assert_eq!(def.disallowed_tools, vec!["file_write".to_string()]);
    assert_eq!(
        def.subagents,
        vec![SubagentEntry::AgentId("researcher".to_string())]
    );
    assert_eq!(def.source, DefinitionSource::CustomRegistry);
}

#[test]
fn definition_from_registry_entry_wildcard_allowlist_round_trips() {
    let mut entry = custom_entry("wildcard_agent");
    entry.tool_allowlist = vec!["*".to_string()];
    let def = definition_from_registry_entry(&entry);
    assert!(matches!(def.tools, ToolScope::Wildcard));

    // Round trip back through the forward direction should reproduce the
    // same wildcard shape `default_entry_from_definition` would emit.
    assert_eq!(tools_to_allowlist(&def.tools, &[]), vec!["*".to_string()]);
}

#[test]
fn definition_from_registry_entry_empty_allowlist_stays_tool_less() {
    // Regression test (P1 review comment on this PR): an empty
    // `tool_allowlist` means "no tools selected" in the settings UI/schema
    // — it must synthesize a `ToolScope::Named(vec![])`, NEVER
    // `ToolScope::Wildcard`. Collapsing empty to Wildcard would silently
    // grant every enabled tool to a custom agent saved with no tools
    // selected, bypassing the least-privilege setting the editor shows.
    let mut entry = custom_entry("tool_less_agent");
    entry.tool_allowlist = Vec::new();
    let def = definition_from_registry_entry(&entry);

    assert!(
        matches!(def.tools, ToolScope::Named(ref names) if names.is_empty()),
        "an empty tool_allowlist must synthesize a tool-less Named([]) scope, not Wildcard: {:?}",
        def.tools
    );

    // Round trip back through the forward direction must reproduce the
    // same empty shape, not `["*"]`.
    assert_eq!(tools_to_allowlist(&def.tools, &[]), Vec::<String>::new());
}

#[test]
fn entry_to_definition_to_entry_round_trip_preserves_key_fields() {
    let entry = custom_entry("finance_analyst");
    let def = definition_from_registry_entry(&entry);

    // Rebuild an entry from the synthesized definition the same way
    // `default_entry_from_definition` does, and confirm the
    // execution-critical fields survive the round trip.
    let roundtripped = AgentRegistryEntry {
        id: def.id.clone(),
        name: def.display_name().to_string(),
        description: def.when_to_use.clone(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: model_to_registry_value(&def.model),
        system_prompt: None,
        tool_allowlist: tools_to_allowlist(&def.tools, &def.extra_tools),
        tool_denylist: def.disallowed_tools.clone(),
        subagents: AgentSubagentPolicy::from_allowlist(
            def.subagents
                .iter()
                .filter_map(|s| match s {
                    SubagentEntry::AgentId(id) => Some(id.clone()),
                    SubagentEntry::Skills(_) => None,
                })
                .collect(),
        ),
        tags: Vec::new(),
        metadata: Value::Null,
    };

    assert_eq!(roundtripped.id, entry.id);
    assert_eq!(roundtripped.name, entry.name);
    assert_eq!(roundtripped.description, entry.description);
    assert_eq!(roundtripped.model, entry.model);
    assert_eq!(roundtripped.tool_allowlist, entry.tool_allowlist);
    assert_eq!(roundtripped.tool_denylist, entry.tool_denylist);
    assert_eq!(roundtripped.subagents, entry.subagents);
}
