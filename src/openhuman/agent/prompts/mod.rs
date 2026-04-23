pub mod types;
pub use types::*;
mod connected_identities;
pub use connected_identities::render_connected_identities;

mod builder;
mod sections;
mod subagent;
mod workspace_files;

pub use builder::{ArchetypePromptSection, DynamicPromptSection, SystemPromptBuilder};
pub use sections::{
    DateTimeSection, IdentitySection, RuntimeSection, SafetySection, ToolsSection,
    UserFilesSection, UserMemorySection, WorkspaceSection,
    render_datetime, render_identity, render_runtime, render_safety, render_tools,
    render_user_files, render_user_memory, render_workspace,
};
pub use subagent::{render_subagent_system_prompt, render_subagent_system_prompt_with_format};

#[cfg(test)]
pub(crate) use std::path::Path;
#[cfg(test)]
pub(crate) use workspace_files::{
    default_workspace_file_content, inject_workspace_file, sync_workspace_file,
};

#[cfg(test)]
mod tests;
