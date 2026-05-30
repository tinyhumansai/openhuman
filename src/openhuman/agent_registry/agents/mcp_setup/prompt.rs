//! System prompt builder for the `mcp_setup` built-in agent.
//!
//! Straightforward: render the static archetype, then append the
//! tool block so the model sees the five `mcp_setup_*` tool schemas
//! plus `ask_user_clarification` (filtered down by the harness from the
//! `agent.toml` allowlist).

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

    fn empty_ctx() -> PromptContext<'static> {
        static EMPTY_VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        let visible = EMPTY_VISIBLE.get_or_init(HashSet::new);
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "mcp_setup",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: visible,
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
            workflows: &[],
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&empty_ctx()).unwrap();
        assert!(!body.is_empty());
        assert!(body.contains("MCP Setup Agent"));
    }

    #[test]
    fn archetype_documents_opaque_ref_invariant() {
        let body = build(&empty_ctx()).unwrap();
        assert!(body.contains("never enters your context"));
        assert!(body.contains("secret://"));
    }

    #[test]
    fn archetype_documents_standard_flow_steps() {
        let body = build(&empty_ctx()).unwrap();
        for needle in [
            "mcp_setup_search",
            "mcp_setup_get",
            "mcp_setup_request_secret",
            "mcp_setup_test_connection",
            "mcp_setup_install_and_connect",
            "ask_user_clarification",
        ] {
            assert!(body.contains(needle), "prompt missing `{needle}`");
        }
    }
}
