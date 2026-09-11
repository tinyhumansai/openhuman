//! System prompt builder for the `workflow_builder` built-in agent (Phase 5a).
//!
//! Assembles the workflow-authoring archetype (`tinyflows_copilot::prompts::WORKFLOW_BUILDER`
//! — it belongs to the engine, not to this host) plus the shared runtime sections (user files, the agent's tool list, and the
//! workspace footer). No `## Safety` block — the agent has `omit_safety_preamble
//! = true` in its TOML because every tool in scope is propose-or-read and has no
//! real external effect (the "propose, never persist" invariant lives in the
//! archetype body instead).

use crate::openhuman::agent::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = tinyflows_copilot::prompts::WORKFLOW_BUILDER;

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(8192);
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
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
