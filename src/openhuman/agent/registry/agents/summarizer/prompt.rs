//! System prompt builder for the `summarizer` built-in agent.
//!
//! Returns the fully-assembled system prompt. Each agent's `build()`
//! composes section helpers from [`crate::openhuman::agent::context::prompt`]
//! in the order it wants — so the output IS what the LLM sees, no
//! post-processing in the runner.

use crate::openhuman::agent::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

/// The summarizer archetype, verbatim.
///
/// `pub` since issue #6014, for the same reason
/// [`payload_summarizer`](crate::openhuman::agent::tinyagents::payload_summarizer)
/// is: the trait invites an embedder to supply its own summarizer — the default
/// implementation dispatches a sub-agent, which an embedder may be unable to do
/// — and the archetype is where the extraction contract is actually written
/// down. Without it, anyone taking that invitation has to reinvent the prompt,
/// and will reinvent it worse: the identifier rule, the structural hints that
/// let a caller decide whether to re-fetch, the error-payload and
/// binary-payload edge cases, and the "do not solve the parent task" boundary
/// are all easy to omit and expensive to discover missing.
///
/// [`build`] remains the entry point for the sub-agent path, which additionally
/// wants the user-files, tools and workspace sections. A caller running one
/// tool-less model call wants this and nothing else.
pub const ARCHETYPE: &str = include_str!("prompt.md");

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
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
