//! System prompt builder for the `welcome` built-in agent.
//!
//! Welcome runs onboarding — it surfaces which integrations the user
//! has already connected and pitches the ones that are still pending.
//! Like the orchestrator, it delegates any integration work rather
//! than executing Composio actions directly, so it renders the same
//! delegator-voice block (inlined here rather than shared, so the
//! skill-executor wording stays scoped to `integrations_agent/prompt.rs`).

use crate::openhuman::context::prompt::{
    render_connected_identities, render_tools, render_user_files, render_workspace,
    ConnectedIntegration, PromptContext,
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

    let identities = render_connected_identities();
    if !identities.trim().is_empty() {
        out.push_str(identities.trim_end());
        out.push_str("\n\n");
    }

    let integrations = render_connected_integrations(ctx.connected_integrations);
    if !integrations.trim().is_empty() {
        out.push_str(integrations.trim_end());
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

/// Render welcome's connected-integrations block — a compact list of
/// the toolkits the user has already authorised. Unconnected entries
/// are skipped (welcome's job during onboarding is to pitch them, not
/// to treat them as usable yet).
fn render_connected_integrations(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    if connected.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Connected Integrations\n\n");
    for ci in connected {
        let _ = writeln!(
            out,
            "- **{}** — {}",
            sanitize_bullet(&ci.toolkit),
            sanitize_bullet(&ci.description),
        );
    }
    out
}

/// Normalise a string for safe inclusion in a single markdown bullet:
/// replace newlines/carriage returns with spaces, collapse runs of
/// whitespace, and trim leading/trailing whitespace so a description
/// with embedded linebreaks can't split the bullet.
fn sanitize_bullet(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let mut out = String::with_capacity(replaced.len());
    let mut prev_space = false;
    for ch in replaced.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
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
            agent_id: "welcome",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: EMPTY_VISIBLE.get_or_init(HashSet::new),
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: integrations,
            include_profile: false,
            include_memory_md: false,
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(!body.is_empty());
        assert!(!body.contains("## Connected Integrations"));
    }

    #[test]
    fn build_lists_only_connected_integrations() {
        let integrations = vec![
            ConnectedIntegration {
                toolkit: "gmail".into(),
                description: "Email access.".into(),
                tools: Vec::new(),
                connected: true,
            },
            ConnectedIntegration {
                toolkit: "notion".into(),
                description: "Pitch during onboarding.".into(),
                tools: Vec::new(),
                connected: false,
            },
        ];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("- **gmail**"));
        assert!(!body.contains("- **notion**"));
    }
}
