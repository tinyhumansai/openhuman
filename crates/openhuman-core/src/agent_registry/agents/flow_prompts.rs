//! Shared composition for the bus-backed TinyFlows builtin archetypes.

use anyhow::Result;

use crate::agent::prompts::PromptContext;
use crate::prompts::{render_tools, render_workspace};

/// Combine module-owned archetype text with OpenHuman's scoped runtime sections.
pub(super) fn build_with_archetype(
    ctx: &PromptContext<'_>,
    archetype_text: &str,
) -> Result<String> {
    let mut out = String::with_capacity(archetype_text.len() + 8192);
    out.push_str(archetype_text.trim_end());
    out.push_str("\n\n");

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    Ok(out)
}
