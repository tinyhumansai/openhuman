//! Scoped async preparation for typed sub-agent prompt rendering.

use super::{AgentDefinition, PromptContext, SubagentRunError};
use crate::agent::subagent_host::tool_prep::load_prompt_source;

/// One spawn's snapshot of the host config, loaded once by [`super::run_subagent`].
///
/// Threading it in as a parameter keeps all prompt, model and context decisions
/// on the same profile snapshot without giving the generic runtime file access.
pub(super) type LoadedConfig = Result<std::sync::Arc<crate::config::Config>, String>;

/// Load and render a typed sub-agent prompt using this spawn's config snapshot.
pub(super) async fn load_typed_prompt_source(
    definition: &AgentDefinition,
    _config: &LoadedConfig,
    mut prompt_context: PromptContext<'_>,
) -> Result<String, SubagentRunError> {
    #[cfg(feature = "flows")]
    let flow_archetype_text = crate::agent::prompts::flow_archetype::load_for_definition(
        _config.as_ref().ok().map(|config| config.as_ref()),
        definition,
    )
    .await?;
    #[cfg(not(feature = "flows"))]
    let flow_archetype_text: Option<String> = None;

    prompt_context.flow_archetype_text = flow_archetype_text.as_deref();
    load_prompt_source(definition, &prompt_context)
}
