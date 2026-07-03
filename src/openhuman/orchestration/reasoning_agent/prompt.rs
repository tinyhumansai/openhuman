//! System prompt builder for the `reasoning_agent` built-in.
//!
//! Assembled per cycle: the base archetype + the active subconscious steering
//! directive (from the [`super::ORCHESTRATION_STEERING`] task-local, or
//! [`super::DEFAULT_STEERING`] when none is set) + tool/safety/workspace context.

use crate::openhuman::context::prompt::{
    render_safety, render_tools, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(6144);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    // Per-cycle steering directive — the load-bearing seam (spec §3.2).
    let steering = super::current_steering()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| super::DEFAULT_STEERING.to_string());
    out.push_str("## Active steering directive\n\n");
    out.push_str(steering.trim());
    out.push_str("\n\n");

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let safety = render_safety();
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

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
    use crate::openhuman::context::prompt::{
        ConnectedIntegration, LearnedContextData, PromptTool, ToolCallFormat,
    };
    use std::collections::HashSet;

    /// Build the prompt with an empty context (mirrors the loader test helper).
    fn build_prompt() -> String {
        let empty_tools: Vec<PromptTool<'_>> = Vec::new();
        let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
        let empty_visible: HashSet<String> = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "reasoning_agent",
            tools: &empty_tools,
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &empty_visible,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &empty_integrations,
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
        };
        build(&ctx).expect("prompt builds")
    }

    #[tokio::test]
    async fn active_steering_directive_is_woven_into_the_prompt() {
        let body =
            super::super::with_steering("PRIORITIZE THE LAUNCH DEADLINE".to_string(), async {
                build_prompt()
            })
            .await;
        assert!(
            body.contains("PRIORITIZE THE LAUNCH DEADLINE"),
            "the per-cycle steering directive must appear in the system prompt"
        );
    }

    #[test]
    fn default_alignment_used_when_no_steering() {
        let body = build_prompt();
        assert!(
            body.contains(super::super::DEFAULT_STEERING),
            "the default alignment directive must be used when no steering is active"
        );
    }
}
