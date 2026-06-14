pub mod types;
pub use types::*;
mod connected_identities;
pub use connected_identities::render_connected_identities;

pub mod builder;
pub use builder::{SystemPromptBuilder, GLOBAL_STYLE_SUFFIX};

pub mod sections;
pub use sections::*;

pub mod render_helpers;
pub use render_helpers::{
    default_workspace_file_content, inject_inline_content, inject_snapshot_content,
    inject_workspace_file, inject_workspace_file_capped, memory_date_label,
    render_ambient_environment, render_datetime, render_grounding, render_identity, render_runtime,
    render_safety, render_subagent_system_prompt, render_subagent_system_prompt_with_format,
    render_tools, render_user_files, render_user_identity, render_user_memory,
    render_user_reflections, render_workspace, sync_workspace_file,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
