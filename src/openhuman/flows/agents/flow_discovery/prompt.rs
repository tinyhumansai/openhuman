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

use crate::openhuman::context::prompt::{
    render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

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
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn ctx() -> PromptContext<'static> {
        static VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        let visible = VISIBLE.get_or_init(HashSet::new);
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "flow_discovery",
            tools: &[],
            workflows: &[],
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
            agents_md_global: None,
            agents_md_local: None,
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&ctx()).unwrap();
        assert!(!body.is_empty());
    }

    #[test]
    fn prompt_teaches_the_read_only_emit_invariant() {
        let body = build(&ctx()).unwrap();
        let lc = body.to_lowercase();
        assert!(lc.contains("suggest_workflows"), "must name the emit tool");
        assert!(
            lc.contains("read-only") || lc.contains("never act") || lc.contains("never build"),
            "prompt must teach the read-only invariant"
        );
    }
}
