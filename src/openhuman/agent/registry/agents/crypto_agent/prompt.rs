//! System prompt builder for the `crypto_agent` built-in agent.
//!
//! Crypto Agent is a narrow-scope, write-capable specialist. The body
//! is the archetype's read/simulate/confirm/execute contract, followed
//! by the standard tool + workspace blocks so the model sees the
//! `wallet_*` / `stock_*` schemas the runtime injected. Identity,
//! skills catalogue and global memory context are omitted — they would
//! dilute the financial-safety voice the archetype establishes.

use crate::openhuman::agent::context::prompt::{
    render_safety, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    tracing::debug!(
        agent_id = ctx.agent_id,
        model = ctx.model_name,
        tool_count = ctx.tools.len(),
        workflow_count = ctx.workflows.len(),
        "[agent_prompt][crypto_agent] build_start"
    );

    let mut out = String::with_capacity(8192);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let user_files = render_user_files(ctx)?;
    let user_files_present = !user_files.trim().is_empty();
    if user_files_present {
        out.push_str(user_files.trim_end());
        out.push_str("\n\n");
    }

    let tools = render_tools(ctx)?;
    let tools_present = !tools.trim().is_empty();
    if tools_present {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let safety = render_safety();
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

    let workspace = render_workspace(ctx)?;
    let workspace_present = !workspace.trim().is_empty();
    if workspace_present {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    tracing::trace!(
        agent_id = ctx.agent_id,
        prompt_len = out.len(),
        user_files_present,
        tools_present,
        workspace_present,
        "[agent_prompt][crypto_agent] build_done"
    );
    Ok(out)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
