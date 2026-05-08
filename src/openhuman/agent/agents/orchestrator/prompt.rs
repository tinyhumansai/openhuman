//! System prompt builder for the `orchestrator` built-in agent.
//!
//! The orchestrator follows a direct-first policy: respond directly or use
//! cheap direct tools whenever possible, and delegate only for specialised
//! execution. It never executes Composio actions itself; the integration
//! block points to `delegate_{toolkit}` tools (synthesised by
//! `orchestrator_tools::collect_orchestrator_tools`) for true
//! external-service operations. That prose lives here (not in the shared
//! prompts module) so the skill-executor voice stays in
//! `integrations_agent/prompt.rs` and nobody has to branch on `agent_id`
//! in a shared section impl.

use crate::openhuman::context::prompt::{
    render_datetime, render_tools, render_user_files, render_workspace, ConnectedIntegration,
    PromptContext,
};
use crate::openhuman::tools::orchestrator_tools::sanitise_slug;
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

/// Render the delegator-voice `## Connected Integrations` block. Only
/// toolkits the user has actively connected are listed — unauthorised
/// toolkits are hidden so the orchestrator cannot hallucinate a delegation
/// to an integration whose `delegate_*` tool does not actually exist.
/// When every toolkit is unconnected the whole section is omitted.
///
/// The tool name printed in the prompt is derived with the same
/// `sanitise_slug` function that `collect_orchestrator_tools` uses when
/// synthesising the real tool objects, so the names in the prompt always
/// match the names in the function-calling schema.
fn render_delegation_guide(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    tracing::debug!(
        total_integrations = integrations.len(),
        connected_count = connected.len(),
        "[delegation-guide] rendering integration section ({} connected / {} total)",
        connected.len(),
        integrations.len()
    );
    if connected.is_empty() {
        tracing::debug!("[delegation-guide] section omitted — no connected integrations");
        return String::new();
    }
    let mut out = String::from(
        "## Connected Integrations\n\n\
         Delegate tasks for these services using the matching `delegate_{toolkit}` tool:\n\n",
    );
    for ci in connected {
        // Use the same slug canonicalisation as `collect_orchestrator_tools`
        // so the tool name in the prompt always matches the synthesised tool.
        let slug = sanitise_slug(&ci.toolkit);
        let _ = writeln!(
            out,
            "- **{}** (delegate via `delegate_{}`): {}",
            ci.toolkit, slug, ci.description
        );
    }
    tracing::debug!(
        section_len = out.len(),
        "[delegation-guide] section emitted ({} bytes)",
        out.len()
    );
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
            curated_snapshot: None,
            user_identity: None,
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(!body.is_empty());
        assert!(!body.contains("## Connected Integrations"));
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
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("delegate_gmail"));
        // Must NOT contain the old verbose spawn_subagent snippet.
        assert!(!body.contains("spawn_subagent(agent_id=\"integrations_agent\""));
        // Delegator voice must NOT use the skill-executor wording.
        assert!(!body.contains("You have direct access"));
    }

    #[test]
    fn delegation_guide_uses_compact_delegate_format() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email access.".into(),
            tools: Vec::new(),
            connected: true,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("delegate_gmail"));
        // Must NOT contain the old verbose spawn_subagent snippet.
        assert!(!body.contains("spawn_subagent(agent_id=\"integrations_agent\""));
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
        assert!(!body.contains("## Connected Integrations"));
    }
}
