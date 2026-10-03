//! Bundled prompt assets and system-prompt rendering pipeline. See
//! [`README.md`](README.md) for the section/builder model and the
//! `agent::prompts`.

pub mod types;
pub use types::*;
mod connected_identities;
pub use connected_identities::render_connected_identities;

pub mod agents_md;
pub use agents_md::{load_agents_md, load_agents_md_layers, AgentsMdContent, AGENTS_MD_FILENAME};

pub mod builder;
pub use builder::{SystemPromptBuilder, TieredPrompt, GLOBAL_STYLE_SUFFIX};

pub mod sections;
pub use sections::*;

pub mod render_helpers;
pub use render_helpers::{
    current_datetime_line, default_workspace_file_content, inject_inline_content,
    inject_workspace_file, inject_workspace_file_capped, render_ambient_environment,
    render_datetime, render_identity, render_runtime, render_safety, render_subagent_system_prompt,
    render_subagent_system_prompt_with_format, render_tools, render_user_identity,
    render_workspace, sync_workspace_file,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
