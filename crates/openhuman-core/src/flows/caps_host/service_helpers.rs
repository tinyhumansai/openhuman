//! Small value adapters shared by the bus callback methods.

use std::sync::Arc;

use tinyflows_bus::runs;

use super::super::binding::CapsHostBinding;
use super::super::OpenHumanMemory;

pub(super) fn memory(binding: &CapsHostBinding) -> OpenHumanMemory {
    OpenHumanMemory {
        ctx: binding.ctx.clone(),
        config: Arc::clone(&binding.config),
        security: Arc::clone(&binding.security),
        approval: binding.approval.clone(),
    }
}

pub(super) fn bus_error(error: impl std::fmt::Display) -> tinybus::Error {
    tinybus::Error::failed(error.to_string())
}

pub(super) fn to_bus_agent_definition(
    definition: &crate::agent_registry::host_definition::AgentDefinition,
) -> Result<runs::AgentDefinition, String> {
    use tinyagents_definition::{ModelSpec, PromptSource, ToolScope};

    let instructions = match &definition.system_prompt {
        PromptSource::Inline(prompt) | PromptSource::Verbatim(prompt) => Some(prompt.clone()),
        PromptSource::File { .. } | PromptSource::Dynamic(()) => None,
    };
    let (model, provider) = match &definition.model {
        ModelSpec::Inherit => (None, None),
        ModelSpec::Exact(model) => (Some(model.clone()), None),
        ModelSpec::Hint(hint) => (Some(format!("hint:{hint}")), None),
    };
    let tools = match &definition.tools {
        ToolScope::Wildcard => Vec::new(),
        ToolScope::Named(names) => names
            .iter()
            .map(|slug| runs::ToolGrant {
                slug: slug.clone(),
                connection_ref: None,
            })
            .collect(),
    };
    Ok(runs::AgentDefinition {
        id: definition.id.clone(),
        name: definition
            .display_name
            .clone()
            .unwrap_or_else(|| definition.id.clone()),
        description: definition.when_to_use.clone(),
        instructions,
        model,
        provider,
        working_dir: None,
        limits: runs::AgentLimits {
            max_steps: Some(u32::try_from(definition.max_iterations).unwrap_or(u32::MAX)),
            max_tool_calls: None,
            agent_timeout_secs: definition
                .timeout_secs
                .map(|secs| u32::try_from(secs).unwrap_or(u32::MAX)),
            tool_timeout_secs: None,
        },
        tools,
        context: Vec::new(),
        metadata: Default::default(),
    })
}
