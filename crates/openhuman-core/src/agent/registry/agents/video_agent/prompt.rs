//! System prompt builder for the `video_agent` built-in agent.
//!
//! Returns the final, fully-assembled system prompt — archetype body (from the
//! sibling `prompt.md`) plus the shared section helpers every agent uses.

use crate::agent::prompts::{render_tools, render_workspace, PromptContext};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
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

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
