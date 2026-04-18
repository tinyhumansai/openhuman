//! System prompt builder for the `critic` built-in agent.
//!
//! Returns the final, fully-assembled system prompt — archetype body
//! (from the sibling `prompt.md`) plus the same section helpers the
//! runtime uses for every other agent.

use crate::openhuman::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
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
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    #[test]
    fn build_returns_nonempty_body() {
        let visible: HashSet<String> = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "critic",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let body = build(&ctx).unwrap();
        assert!(!body.is_empty());
    }
}
