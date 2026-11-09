//! Bundled prompt assets and OpenHuman-specific agent prompt adapters.

pub mod types;
pub use types::*;
#[cfg(feature = "flows")]
pub(crate) mod flow_archetype;
pub use crate::integrations::render_connected_identities;

pub mod agents_md;
pub use agents_md::{load_agents_md, load_agents_md_layers};

pub use crate::prompts::composition::{SystemPromptBuilder, TieredPrompt, GLOBAL_STYLE_SUFFIX};

#[path = "render_helpers/subagent.rs"]
mod subagent;
pub use subagent::{render_subagent_system_prompt, render_subagent_system_prompt_with_format};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
