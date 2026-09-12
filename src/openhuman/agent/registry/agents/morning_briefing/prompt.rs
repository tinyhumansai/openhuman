//! System prompt builder for the `morning_briefing` built-in agent.
//!
//! Returns the fully-assembled system prompt. Each agent's `build()`
//! composes section helpers from [`crate::openhuman::agent::context::prompt`]
//! in the order it wants — so the output IS what the LLM sees, no
//! post-processing in the runner.

use crate::openhuman::agent::context::prompt::{
    render_ambient_environment, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let user_files = render_user_files(ctx)?;
    if !user_files.trim().is_empty() {
        out.push_str(user_files.trim_end());
        out.push_str("\n\n");
    }

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push_str("\n\n");
    }

    // Ambient runtime + user identity + current date/time so the
    // briefing agent stops asking the user "what timezone are you in?"
    // when the desktop app already knows — issue #926. Block sits at
    // the prompt tail because the embedded `Local::now()` makes it
    // time-volatile, matching the KV cache convention from
    // `SystemPromptBuilder::with_defaults`.
    let ambient = render_ambient_environment(ctx)?;
    if !ambient.trim().is_empty() {
        out.push_str(ambient.trim_end());
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
