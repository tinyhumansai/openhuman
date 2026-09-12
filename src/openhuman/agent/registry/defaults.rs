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
#[path = "defaults_tests.rs"]
mod tests;
