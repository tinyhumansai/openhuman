//! System prompt builder for the `flow_discovery` built-in agent (the "Flow
//! Scout").
//!
//! Assembles the discovery archetype (from the sibling `prompt.md`) plus the
//! shared runtime sections (user files, the agent's tool list, and the
//! workspace footer). PROFILE.md / MEMORY.md are injected by the harness per the
//! agent's `omit_profile = false` / `omit_memory_md = false` TOML flags — the
//! scout grounds its suggestions in who the user is, so it reads them directly.
//! No `## Safety` block: `omit_safety_preamble = true` because every tool in
//! scope is read-only except the `suggest_workflows` emit sink (which has no
//! external effect); the "read, then suggest — never act" invariant lives in
//! the archetype body instead.

use crate::openhuman::agent::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = tinyflows_copilot::prompts::FLOW_DISCOVERY;

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
