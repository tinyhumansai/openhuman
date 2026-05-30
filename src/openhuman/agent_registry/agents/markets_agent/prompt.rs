//! System prompt builder for the `markets_agent` built-in agent.
//!
//! Markets Agent is a narrow-scope, write-capable specialist for
//! prediction-market and event-contract trading on Polymarket and
//! Kalshi. The body is the archetype's read/propose/confirm/execute
//! contract, followed by the standard tool + workspace blocks so the
//! model sees the `polymarket` / `kalshi` schemas the runtime injected.
//! Identity, skills catalogue and global memory context are omitted —
//! they would dilute the financial-safety voice the archetype
//! establishes.

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
        skill_count = ctx.skills.len(),
        "[agent_prompt][markets_agent] build_start"
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
        "[agent_prompt][markets_agent] build_done"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn empty_ctx() -> PromptContext<'static> {
        use std::sync::OnceLock;
        static EMPTY_VISIBLE: OnceLock<HashSet<String>> = OnceLock::new();
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "markets_agent",
            tools: &[],
            skills: &[],
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
            workflows: &[],
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&empty_ctx()).unwrap();
        assert!(!body.is_empty());
        assert!(body.contains("Markets Agent"));
    }

    #[test]
    fn build_enforces_read_propose_confirm_execute() {
        let body = build(&empty_ctx()).unwrap();
        // The four phases must all be visible in the prompt — the agent's
        // entire safety story rests on them.
        assert!(
            body.contains("read, simulate, confirm, then execute")
                || body.contains("read/propose/confirm/execute"),
            "prompt must spell out the read→propose→confirm→execute contract"
        );
        assert!(
            body.contains("ask_user_clarification"),
            "prompt must require explicit user confirmation before execute"
        );
        assert!(
            body.contains("approved=true"),
            "prompt must require the venue-level approved=true flag for write actions"
        );
    }

    #[test]
    fn build_forbids_fabrication_and_logging_secrets() {
        let body = build(&empty_ctx()).unwrap();
        assert!(
            body.contains("No fabrication"),
            "prompt must explicitly forbid fabricating ticker / market / price params"
        );
        assert!(
            body.contains("Never log secrets") || body.contains("never log secrets"),
            "prompt must forbid echoing API keys / signing secrets"
        );
    }

    #[test]
    fn build_distinguishes_from_crypto_agent() {
        let body = build(&empty_ctx()).unwrap();
        assert!(
            body.contains("crypto_agent"),
            "prompt must point on-chain work to crypto_agent so concerns stay separated"
        );
    }
}
