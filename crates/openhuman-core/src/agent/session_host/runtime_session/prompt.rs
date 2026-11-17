//! Loads the module-owned prompt fragment for a cold TinyFlows session turn.

use anyhow::Result;

use super::OpenHumanTurnPrelude;

pub(super) async fn flow_archetype_text(
    prelude: &OpenHumanTurnPrelude,
    cold: bool,
) -> Result<Option<String>> {
    if !cold {
        return Ok(None);
    }

    #[cfg(feature = "flows")]
    {
        match prelude.definition.as_deref() {
            Some(definition) => {
                crate::agent::prompts::flow_archetype::load_for_definition(
                    prelude.runtime_config.as_deref(),
                    definition,
                )
                .await
            }
            None => Ok(None),
        }
    }
    #[cfg(not(feature = "flows"))]
    {
        let _ = prelude;
        Ok(None)
    }
}
