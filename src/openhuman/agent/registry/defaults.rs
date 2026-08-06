//! Default registry entries derived from shipped harness definitions.

use serde_json::Value;

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentTier, DefinitionSource, IterationPolicy, ModelSpec, PromptSource,
    SandboxMode, SubagentEntry, ToolScope, TriggerMemoryAgent,
};

use super::types::{AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy};

pub fn default_agents() -> Vec<AgentRegistryEntry> {
    crate::openhuman::agent::registry::agents::load_builtins()
        .map(|defs| {
            defs.into_iter()
                .map(default_entry_from_definition)
                .collect()
        })
        .unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "[agent_registry] failed to load default agent definitions"
            );
            Vec::new()
        })
}

fn default_entry_from_definition(def: AgentDefinition) -> AgentRegistryEntry {
    AgentRegistryEntry {
        id: def.id.clone(),
        name: def.display_name().to_string(),
        description: def.when_to_use,
        source: AgentRegistrySource::Default,
        enabled: true,
        model: model_to_registry_value(&def.model),
        system_prompt: None,
        tool_allowlist: tools_to_allowlist(&def.tools, &def.extra_tools),
        tool_denylist: def.disallowed_tools,
        subagents: AgentSubagentPolicy::from_allowlist(
            def.subagents
                .into_iter()
                .filter_map(|entry| match entry {
                    SubagentEntry::AgentId(id) => Some(id),
                    SubagentEntry::Skills(_) => None,
                })
                .collect(),
        ),
        tags: vec![def.agent_tier.as_str().to_string()],
        metadata: Value::Null,
    }
}

/// Inverse of [`default_entry_from_definition`] — synthesizes a harness
/// [`AgentDefinition`] from a user-authored [`AgentRegistryEntry`] so a
/// custom agent (one with no shipped harness definition) can be built through
/// the same [`crate::openhuman::agent::Agent::from_config_for_agent`] factory
/// path as a built-in, and therefore run with its real tool belt rather than
/// degrade to a persona-only completion.
///
/// Mapping (mirrors `default_entry_from_definition` field-for-field, in
/// reverse):
/// * `description` -> `when_to_use`; `name` -> `display_name`.
/// * `system_prompt` -> `PromptSource::Inline` (empty string when unset,
///   which renders as an empty subagent body rather than erroring).
/// * `model` -> `ModelSpec` via [`registry_value_to_model_spec`], the
///   inverse of [`model_to_registry_value`].
/// * `tool_allowlist` -> `ToolScope` via [`allowlist_to_tool_scope`]: exactly
///   `["*"]` means `Wildcard`; an empty list means `Named(vec![])` (no
///   tools) — matching how `tools_to_allowlist` renders `Wildcard` as
///   `["*"]` and `Named(vec![])` as `[]`.
/// * `tool_denylist` -> `disallowed_tools` (direct clone).
/// * `subagents.allowlist` -> one `SubagentEntry::AgentId` per entry.
///
/// Every other field is a harness-side concern a custom agent never
/// authors today, so it takes the harness's own safe default: `omit_* =
/// true` (narrow/lean prompt, matching every non-orchestrator built-in),
/// `temperature = 0.4`, `max_iterations = 8` under `IterationPolicy::Strict`,
/// `sandbox_mode = None`, `agent_tier = Worker`. `source` is stamped
/// [`DefinitionSource::CustomRegistry`] so it's visibly distinct from a
/// shipped/TOML-file definition in logs and `agent::list_definitions`.
pub fn definition_from_registry_entry(entry: &AgentRegistryEntry) -> AgentDefinition {
    AgentDefinition {
        id: entry.id.clone(),
        when_to_use: entry.description.clone(),
        display_name: Some(entry.name.clone()),
        system_prompt: PromptSource::Inline(entry.system_prompt.clone().unwrap_or_default()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: registry_value_to_model_spec(entry.model.as_deref()),
        temperature: 0.4,
        tools: allowlist_to_tool_scope(&entry.tool_allowlist),
        disallowed_tools: entry.tool_denylist.clone(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 8,
        iteration_policy: IterationPolicy::Strict,
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: TriggerMemoryAgent::Never,
        tokenjuice_compression: Default::default(),
        subagents: entry
            .subagents
            .allowlist
            .iter()
            .cloned()
            .map(SubagentEntry::AgentId)
            .collect(),
        delegate_name: None,
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::CustomRegistry,
        graph: Default::default(),
    }
}

/// Inverse of [`model_to_registry_value`]: `None`/`"inherit"` -> `Inherit`;
/// `"hint:<role>"` -> `Hint(role)`; anything else -> `Exact(value)`.
fn registry_value_to_model_spec(value: Option<&str>) -> ModelSpec {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => ModelSpec::Inherit,
        Some("inherit") => ModelSpec::Inherit,
        Some(v) => match v.strip_prefix("hint:") {
            Some(hint) => ModelSpec::Hint(hint.to_string()),
            None => ModelSpec::Exact(v.to_string()),
        },
    }
}

/// Inverse of [`tools_to_allowlist`]'s `Wildcard` rendering: exactly `["*"]`
/// means "all tools" (`ToolScope::Wildcard`). An **empty** allowlist is a
/// deliberate `ToolScope::Named(vec![])` — i.e. tool-less — matching what the
/// settings UI/schema mean by "no tools selected", and matching the forward
/// direction: `tools_to_allowlist(&ToolScope::Named(vec![]), &[])` already
/// renders back to `[]`, never `["*"]`. Collapsing empty to `Wildcard` here
/// would silently grant a custom agent saved with no tools selected every
/// enabled tool, bypassing the least-privilege setting the editor shows.
fn allowlist_to_tool_scope(allowlist: &[String]) -> ToolScope {
    if allowlist == ["*"] {
        ToolScope::Wildcard
    } else {
        ToolScope::Named(allowlist.to_vec())
    }
}

fn model_to_registry_value(model: &ModelSpec) -> Option<String> {
    match model {
        ModelSpec::Inherit => Some("inherit".to_string()),
        ModelSpec::Exact(value) => Some(value.clone()),
        ModelSpec::Hint(value) => Some(format!("hint:{value}")),
    }
}

fn tools_to_allowlist(scope: &ToolScope, extra_tools: &[String]) -> Vec<String> {
    let mut tools = match scope {
        ToolScope::Wildcard => vec!["*".to_string()],
        ToolScope::Named(names) => names.clone(),
    };
    for tool in extra_tools {
        if !tools.contains(tool) {
            tools.push(tool.clone());
        }
    }
    tools
}

#[cfg(test)]
mod tests {
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
}
