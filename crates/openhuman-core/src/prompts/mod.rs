//! Prompt product copy and rendering helpers used by agent and channel prompts.

pub mod composition;
pub mod sections;
pub use composition::{SystemPromptBuilder, TieredPrompt, GLOBAL_STYLE_SUFFIX};
pub use section_renderers::{
    current_datetime_line_at, register_prompt_clock, PromptClockRegistration,
};

#[path = "section_renderers.rs"]
mod section_renderers;
#[path = "workspace_files.rs"]
mod workspace_files;

pub(crate) use section_renderers::*;
pub(crate) use workspace_files::*;
