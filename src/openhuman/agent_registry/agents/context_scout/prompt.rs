//! System prompt builder for the `context_scout` built-in agent.
//!
//! The scout is a read-only pre-flight context collector. Its prompt is the
//! role markdown ([`prompt.md`]) followed by the user-file injection
//! (PROFILE.md = goals, MEMORY.md = curated long-term memory — both kept in
//! because grounding the orchestrator in *who the user is and what they want*
//! is the scout's whole job), the scout's own read-only tool catalogue, and
//! the workspace block. The orchestrator's tool catalogue (the tools the scout
//! recommends *back* to the parent) is injected at spawn time by
//! `AgentPrepareContextTool`, not here — this builder only describes the
//! scout's own gathering surface.

use crate::openhuman::context::prompt::{
    render_tools, render_user_files, render_workspace, ConnectedIntegration, PromptContext,
};
use anyhow::Result;
use std::fmt::Write as _;

const ARCHETYPE: &str = include_str!("prompt.md");

/// Render a compact `## Connected Integrations` block listing the platforms
/// the user actually has connected. The scout's role prompt tells it to use
/// this to decide whether a request is serviceable by a connected app, so the
/// builder must emit it (the orchestrator surfaces the same data via its own
/// delegation guide). Empty when nothing is connected.
fn render_connected_integrations(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    tracing::trace!(
        target: "context_scout",
        total = integrations.len(),
        connected = connected.len(),
        "[context_scout] rendering connected integrations block"
    );
    if connected.is_empty() {
        tracing::trace!(
            target: "context_scout",
            "[context_scout] no connected integrations — omitting block"
        );
        return String::new();
    }
    let mut out = String::from(
        "## Connected Integrations\n\nThese platforms are connected and reachable via the \
         orchestrator's delegation tools — factor them into the plan when a request needs \
         live data from one:\n",
    );
    for ci in connected {
        let _ = writeln!(out, "- {}", ci.toolkit);
    }
    out
}

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    tracing::debug!(
        target: "context_scout",
        agent_id = %ctx.agent_id,
        include_profile = ctx.include_profile,
        include_memory_md = ctx.include_memory_md,
        tool_count = ctx.tools.len(),
        "[context_scout] building system prompt"
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

    let integrations = render_connected_integrations(ctx.connected_integrations);
    if !integrations.trim().is_empty() {
        tracing::debug!(
            target: "context_scout",
            "[context_scout] appended connected-integrations section"
        );
        out.push_str(integrations.trim_end());
        out.push_str("\n\n");
    } else {
        tracing::debug!(
            target: "context_scout",
            "[context_scout] no connected-integrations section to append"
        );
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    tracing::debug!(
        target: "context_scout",
        prompt_chars = out.chars().count(),
        "[context_scout] system prompt built"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn test_ctx() -> PromptContext<'static> {
        // Leak a HashSet so the &reference satisfies the 'static-ish lifetime
        // the helper needs in this throwaway test context.
        let visible: &'static HashSet<String> = Box::leak(Box::new(HashSet::new()));
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "context_scout",
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
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&test_ctx()).unwrap();
        assert!(!body.is_empty());
    }

    #[test]
    fn body_describes_the_context_bundle_contract() {
        let body = build(&test_ctx()).unwrap();
        assert!(body.contains("[context_bundle]"));
        assert!(body.contains("has_enough_context"));
        assert!(body.contains("recommended_tool_calls"));
    }

    #[test]
    fn body_instructs_transcript_and_skill_gathering() {
        // The enrichment is only real if the role prompt actually tells the
        // scout to search past chats and recommend skills — lock that wiring.
        let body = build(&test_ctx()).unwrap();
        assert!(
            body.contains("transcript_search"),
            "scout prompt must instruct searching past conversations"
        );
        assert!(
            body.contains("recommended_skills"),
            "scout prompt must define the recommended_skills output block"
        );
        assert!(
            body.contains("list_workflows"),
            "scout prompt must point at skill discovery"
        );
    }

    fn integration(toolkit: &str, connected: bool) -> ConnectedIntegration {
        ConnectedIntegration {
            toolkit: toolkit.to_string(),
            description: String::new(),
            tools: vec![],
            gated_tools: vec![],
            connected,
            connections: vec![],
            non_active_status: None,
        }
    }

    #[test]
    fn render_connected_integrations_lists_only_connected() {
        let out = render_connected_integrations(&[
            integration("gmail", true),
            integration("notion", false),
        ]);
        assert!(out.contains("## Connected Integrations"));
        assert!(out.contains("- gmail"));
        assert!(
            !out.contains("notion"),
            "unconnected toolkits must be omitted"
        );
    }

    #[test]
    fn render_connected_integrations_empty_when_none_connected() {
        assert!(render_connected_integrations(&[integration("gmail", false)]).is_empty());
        assert!(render_connected_integrations(&[]).is_empty());
    }
}
