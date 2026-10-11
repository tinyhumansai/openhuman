//! System prompt builder for the `summarizer` built-in agent.
//!
//! Returns the fully-assembled system prompt. Each agent's `build()`
//! composes section helpers from [`crate::agent::prompts`]
//! in the order it wants — so the output IS what the LLM sees, no
//! post-processing in the runner.

use crate::agent::prompts::{render_tools, render_workspace, PromptContext};
use anyhow::Result;

/// The summarizer archetype: TinyJuice's extraction contract, verbatim.
///
/// TinyJuice owns the tool-output summary — it writes this prompt into every
/// `MlHost.Generate` request it sends — so this is a re-export rather than a
/// second copy that could drift. It stays `pub` (issue #6014) so an embedder
/// supplying its own [`PayloadSummarizer`](crate::agent::tinyagents::payload_summarizer::PayloadSummarizer)
/// can read the contract its model is asked to follow.
pub const ARCHETYPE: &str = tinyjuice_bus::summary::SYSTEM_PROMPT;

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
