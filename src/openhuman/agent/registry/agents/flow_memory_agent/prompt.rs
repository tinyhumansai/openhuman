//! System prompt builder for the `flow_memory_agent` built-in agent.
//!
//! This agent is a read-only context/memory retrieval specialist a flow
//! `agent` node routes to via `config.agent_ref` for any run-time context,
//! style, history, or people need. Its prompt is the role markdown
//! ([`prompt.md`]) followed by the user-file injection (PROFILE.md = goals,
//! MEMORY.md = curated long-term memory — both kept in because grounding
//! answers in *who the user is and what they want* is this agent's whole
//! job), its own read-only tool catalogue, and the workspace block.

use crate::openhuman::agent::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    tracing::debug!(
        target: "flow_memory_agent",
        agent_id = %ctx.agent_id,
        include_profile = ctx.include_profile,
        include_memory_md = ctx.include_memory_md,
        tool_count = ctx.tools.len(),
        "[flow_memory_agent] building system prompt"
    );
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    // PROFILE.md (goals) + MEMORY.md (long-term memory). Gated on
    // `ctx.include_profile` / `ctx.include_memory_md`, which the runner sets
    // from the definition's `omit_profile = false` / `omit_memory_md = false`.
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

    tracing::debug!(
        target: "flow_memory_agent",
        prompt_chars = out.chars().count(),
        "[flow_memory_agent] system prompt built"
    );
    Ok(out)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
