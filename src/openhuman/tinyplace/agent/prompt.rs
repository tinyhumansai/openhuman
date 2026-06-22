//! System prompt builder for the `tinyplace_agent` built-in agent.
//!
//! The tiny.place domain owns this worker because its prompt, allowed tool
//! surface, and future SDK-backed actions should evolve with the domain code.

use crate::openhuman::context::prompt::{
    render_safety, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    tracing::debug!(
        agent_id = ctx.agent_id,
        model = ctx.model_name,
        tool_count = ctx.tools.len(),
        "[agent_prompt][tinyplace_agent] build_start"
    );

    let mut out = String::with_capacity(6144);
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
        "[agent_prompt][tinyplace_agent] build_done"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn empty_ctx() -> PromptContext<'static> {
        static EMPTY_VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "tinyplace_agent",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: EMPTY_VISIBLE.get_or_init(HashSet::new),
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&empty_ctx()).unwrap();
        assert!(!body.is_empty());
        assert!(body.contains("Tinyplace Agent"));
    }

    #[test]
    fn archetype_documents_current_tool_surface() {
        let body = build(&empty_ctx()).unwrap();
        // Capability scope is still described in prose.
        assert!(body.contains("identity registration"));
        assert!(body.contains("encrypted DMs"));
        assert!(body.contains("marketplace trading"));
        assert!(body.contains("x402 payment challenges"));
        // The curated flow surface is named so the agent knows its tools.
        assert!(body.contains("tinyplace_status"));
        assert!(body.contains("tinyplace_graphql"));
        assert!(body.contains("tinyplace_call"));
        assert!(body.contains("tinyplace_help"));
    }
}
