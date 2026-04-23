mod datetime;
mod identity;
mod runtime;
mod safety;
mod tools;
mod user_files;
mod user_memory;
mod workspace;

pub use datetime::{DateTimeSection, render_datetime};
pub use identity::{IdentitySection, render_identity};
pub use runtime::{RuntimeSection, render_runtime};
pub use safety::{SafetySection, render_safety};
pub use tools::{ToolsSection, render_tools};
pub use user_files::{UserFilesSection, render_user_files};
pub use user_memory::{UserMemorySection, render_user_memory};
pub use workspace::{WorkspaceSection, render_workspace};

pub(crate) use tools::render_pformat_signature_for_box_tool;

use crate::openhuman::agent::prompts::types::{
    ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
};
use crate::openhuman::skills::Skill;
use std::sync::OnceLock;

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
pub(super) fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_SKILLS: &[Skill] = &[];
    static EMPTY_INTEGRATIONS: &[ConnectedIntegration] = &[];
    // SAFETY: the &HashSet reference must outlive the returned context;
    // a leaked OnceLock-style allocation gives us a permanent 'static
    // anchor without adding runtime cost on the hot path.
    static EMPTY_VISIBLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(std::collections::HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new(""),
        model_name: "",
        agent_id: "",
        tools: EMPTY_TOOLS,
        skills: EMPTY_SKILLS,
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
    }
}
