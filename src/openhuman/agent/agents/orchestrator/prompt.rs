//! System prompt builder for the `orchestrator` built-in agent.
//!
//! The orchestrator follows a direct-first policy: respond directly or use
//! cheap direct tools whenever possible, and delegate only for specialised
//! execution. It never executes Composio actions itself; the integration
//! block points to `spawn_subagent(integrations_agent, toolkit=…)` for true
//! external-service operations. That prose lives here (not in the shared
//! prompts module) so the skill-executor voice stays in
//! `integrations_agent/prompt.rs` and nobody has to branch on `agent_id`
//! in a shared section impl.

use crate::openhuman::context::prompt::{
    render_datetime, render_tools, render_user_files, render_workspace, ConnectedIntegration,
    PromptContext,
};
use anyhow::Result;
use std::fmt::Write;

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

    let identities = ctx.connected_identities_md.as_str();
    if !identities.trim().is_empty() {
        out.push_str(identities.trim_end());
        out.push_str("\n\n");
    }

    let integrations = render_delegation_guide(ctx.connected_integrations);
    if !integrations.trim().is_empty() {
        out.push_str(integrations.trim_end());
        out.push_str("\n\n");
    }

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let datetime = render_datetime(ctx)?;
    if !datetime.trim().is_empty() {
        out.push_str(datetime.trim_end());
        out.push_str("\n\n");
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    Ok(out)
}

/// Render the delegator-voice `## Delegation Guide — Integrations`
/// block. Only toolkits the user has actively connected are listed —
/// unauthorised toolkits are hidden so the orchestrator can't hallucinate
/// a spawn against an integration the `spawn_subagent` pre-flight will
/// immediately reject. When every toolkit is unconnected, the whole
/// section is omitted.
fn render_delegation_guide(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    if connected.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "## Delegation Guide — Integrations\n\n\
         For any task that touches one of these external services, \
         delegate to `integrations_agent` with the matching `toolkit` \
         argument. The sub-agent receives the full action catalogue \
         for that integration as native tool schemas — do not attempt \
         to call integration actions directly from this agent.\n\n\
         Only the integrations listed below are currently authorised. \
         If the user asks about another service, tell them to connect \
         it in **Skills** page before retrying.\n\n",
    );
    for ci in connected {
        let _ = writeln!(
            out,
            "- **{}** — {}\n  Delegate with: `spawn_subagent(agent_id=\"integrations_agent\", toolkit=\"{}\", prompt=<task>)`",
            ci.toolkit, ci.description, ci.toolkit,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn ctx_with<'a>(integrations: &'a [ConnectedIntegration]) -> PromptContext<'a> {
        use std::sync::OnceLock;
        static EMPTY_VISIBLE: OnceLock<HashSet<String>> = OnceLock::new();
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "orchestrator",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: EMPTY_VISIBLE.get_or_init(HashSet::new),
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: integrations,
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            user_identity: None,
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(!body.is_empty());
        assert!(!body.contains("## Delegation Guide"));
    }

    #[test]
    fn build_includes_datetime() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(body.contains("## Current Date & Time"));
    }

    #[test]
    fn build_includes_direct_first_decision_tree() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(body.contains("## Delegation Decision Tree (Direct-First)"));
        assert!(body.contains(
            "Default bias: **do not spawn a sub-agent when a direct response or direct tool call is sufficient**."
        ));
    }

    #[test]
    fn build_emits_delegation_guide_with_spawn_snippet() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email access.".into(),
            tools: Vec::new(),
            connected: true,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("## Delegation Guide — Integrations"));
        assert!(body.contains(
            "spawn_subagent(agent_id=\"integrations_agent\", toolkit=\"gmail\", prompt=<task>)"
        ));
        // Delegator voice must NOT use the skill-executor wording.
        assert!(!body.contains("You have direct access"));
    }

    #[test]
    fn build_hides_unconnected_integrations() {
        // Only connected toolkits make it into the Delegation Guide
        // — unconnected entries would just trigger a spawn_subagent
        // pre-flight rejection, so keeping them out keeps the prompt
        // focused on what the orchestrator can actually delegate.
        let integrations = vec![
            ConnectedIntegration {
                toolkit: "gmail".into(),
                description: "Email.".into(),
                tools: Vec::new(),
                connected: true,
            },
            ConnectedIntegration {
                toolkit: "linear".into(),
                description: "Tracker.".into(),
                tools: Vec::new(),
                connected: false,
            },
        ];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("- **gmail**"));
        assert!(!body.contains("- **linear**"));
    }

    #[test]
    fn build_omits_guide_when_no_integrations_connected() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "linear".into(),
            description: "Tracker.".into(),
            tools: Vec::new(),
            connected: false,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(!body.contains("## Delegation Guide"));
    }
}
