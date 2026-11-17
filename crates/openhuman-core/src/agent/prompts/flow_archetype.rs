//! Scoped TinyFlows archetype prompt loading for the two flow builtins.

use anyhow::{anyhow, ensure, Result};
use tinyagents_definition::PromptSource;

use crate::{agent_registry::host_definition::AgentDefinition, config::Config};

/// Whether `definition` is one of the bus-backed builtin flow prompt owners.
///
/// User definitions and static/session prompt overrides keep their own source
/// and never cause a TinyFlows module lookup, even when they reuse a builtin id.
pub(crate) fn is_bus_backed_builtin(definition: &AgentDefinition) -> bool {
    prompt_owner(definition).is_some()
}

/// Prompt owner stamped by the builtin loader and preserved by cloned specs.
/// Actor identity may change for embedded agents, so it is not a prompt key.
pub(crate) fn prompt_owner(definition: &AgentDefinition) -> Option<&str> {
    if !matches!(&definition.system_prompt, PromptSource::Dynamic(())) {
        return None;
    }
    let owner = definition.extensions.tinyflows_prompt_owner.as_deref()?;
    matches!(owner, "workflow_builder" | "flow_discovery").then_some(owner)
}

/// Fetch the archetype for a resolved definition through the module bus.
///
/// The caller supplies the exact config snapshot that selected the module
/// generation. No ambient config or fallback prompt is used.
pub(crate) async fn load_for_definition(
    config: Option<&Config>,
    definition: &AgentDefinition,
) -> Result<Option<String>> {
    if !is_bus_backed_builtin(definition) {
        return Ok(None);
    }

    let owner = prompt_owner(definition).expect("bus-backed definitions have an owner");
    let config = config.ok_or_else(|| {
        anyhow!(
            "cannot render TinyFlows builtin `{}` without its scoped runtime config",
            owner
        )
    })?;
    fetch_prompt(config, owner).await.map(Some)
}

/// Fetch one of the two builtin flow archetypes through TinyFlows Copilot.
pub(crate) async fn fetch_prompt(config: &Config, agent_id: &str) -> Result<String> {
    ensure!(
        matches!(agent_id, "workflow_builder" | "flow_discovery"),
        "unsupported TinyFlows builtin prompt `{agent_id}`"
    );
    let proxy = crate::modules::tinyflows::copilot_proxy(config)
        .await
        .map_err(anyhow::Error::msg)
        .map_err(|error| error.context(format!("loading TinyFlows prompt `{agent_id}`")))?;
    let response: tinyflows_bus::copilot::TextResponse = proxy
        .call("GetPrompt", (agent_id.to_string(),))
        .await
        .map_err(|error| anyhow!(error.wire_message()))
        .map_err(|error| error.context(format!("fetching TinyFlows prompt `{agent_id}`")))?;
    ensure!(
        !response.text.trim().is_empty(),
        "TinyFlows returned an empty prompt for builtin `{agent_id}`"
    );
    Ok(response.text)
}

#[cfg(test)]
#[path = "flow_archetype_tests.rs"]
mod tests;
