//! System prompt builder for the `master_agent` built-in.
//!
//! The human-facing archetype + the active subconscious steering directive
//! (reused from [`crate::openhuman::orchestration::reasoning_agent`]) + tool /
//! safety / workspace context. Mirrors the reasoning core's assembly but reads
//! this agent's own [`prompt.md`].

use crate::openhuman::context::prompt::{
    render_safety, render_tools, render_workspace, PromptContext,
};
use crate::openhuman::orchestration::reasoning_agent::{current_steering, DEFAULT_STEERING};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(6144);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    // Per-cycle steering directive — reuses the reasoning core's task-local seam.
    let steering = current_steering()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_STEERING.to_string());
    out.push_str("## Active steering directive\n\n");
    out.push_str(steering.trim());
    out.push_str("\n\n");

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let safety = render_safety();
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    Ok(out)
}
