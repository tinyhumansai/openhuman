//! System prompt builder for the `integrations_agent` built-in agent.
//!
//! `integrations_agent` is the one sub-agent that executes Composio actions
//! directly — every other agent delegates to it via `spawn_subagent`.
//! That means the prompt owns two blocks nobody else renders:
//!
//! * `## Available Skills` — the QuickJS skill catalogue it can invoke
//!   through the runtime.
//! * `## Connected Integrations` — the list of Composio toolkits the
//!   user has connected, framed as "you have direct access to the
//!   action tools in your tool list" rather than "delegate to integrations_agent".
//!
//! Both blocks live here (not in the shared prompts module) so the
//! delegator agents stay lean and the integrations_agent-specific wording
//! isn't a branch on `agent_id` somewhere else.

use crate::openhuman::context::prompt::{
    render_connected_identities, render_safety, render_tools, render_user_files, render_workspace,
    ConnectedIntegration, PromptContext,
};
use crate::openhuman::skills::Skill;
use anyhow::Result;
use std::fmt::Write;
use std::path::Path;

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

    let skills = render_available_skills(ctx.skills, ctx.workspace_dir);
    if !skills.trim().is_empty() {
        out.push_str(skills.trim_end());
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

/// Render the `## Available Skills` XML catalogue of QuickJS skills
/// this agent can invoke through the host runtime. Empty when no skills
/// are registered.
fn render_available_skills(skills: &[Skill], workspace_dir: &Path) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Available Skills\n\n<available_skills>\n");
    for skill in skills {
        let location = skill.location.clone().unwrap_or_else(|| {
            workspace_dir
                .join("skills")
                .join(&skill.name)
                .join("SKILL.md")
        });
        let _ = writeln!(
            out,
            "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n    <location>{}</location>\n  </skill>",
            xml_escape(&skill.name),
            xml_escape(&skill.description),
            xml_escape(&location.display().to_string()),
        );
    }
    out.push_str("</available_skills>");
    out
}

/// Escape XML-sensitive characters so skill metadata can't break the
/// surrounding `<available_skills>` block if a name or description
/// contains `<`, `>`, or `&`.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render the skill-executor-flavoured `## Connected Integrations`
/// block. Tells the model that the action tools for each toolkit are
/// already in its tool list and to call them directly — no delegation
/// wording, because `integrations_agent` IS the delegation target.
fn render_connected_integrations(integrations: &[ConnectedIntegration]) -> String {
    let connected: Vec<&ConnectedIntegration> =
        integrations.iter().filter(|ci| ci.connected).collect();
    if connected.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "## Connected Integrations\n\n\
         You have direct access to the following external services. \
         The corresponding action tools are in your tool list with \
         their typed parameter schemas — call them by name.\n\n",
    );
    for ci in connected {
        let _ = writeln!(out, "- **{}** — {}", ci.toolkit, ci.description);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn ctx_with<'a>(
        integrations: &'a [ConnectedIntegration],
        skills: &'a [Skill],
    ) -> PromptContext<'a> {
        // Leak a HashSet so the returned context borrows a 'static-ish
        // reference — the test owns the value for its lifetime.
        use std::sync::OnceLock;
        static EMPTY_VISIBLE: OnceLock<HashSet<String>> = OnceLock::new();
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "integrations_agent",
            tools: &[],
            skills,
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
        let body = build(&ctx_with(&[], &[])).unwrap();
        assert!(!body.is_empty());
        assert!(!body.contains("## Connected Integrations"));
        assert!(!body.contains("## Available Skills"));
    }

    #[test]
    fn build_includes_connected_integrations_in_executor_voice() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email access.".into(),
            tools: Vec::new(),
            connected: true,
        }];
        let body = build(&ctx_with(&integrations, &[])).unwrap();
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("You have direct access"));
        assert!(body.contains("- **gmail** — Email access."));
        // `integrations_agent` must NOT render the delegator spawn snippet —
        // that belongs on the orchestrator/welcome side.
        assert!(!body.contains("Delegation Guide"));
        assert!(!body.contains("spawn_subagent"));
    }

    #[test]
    fn build_skips_unconnected_integrations() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "notion".into(),
            description: "Pages.".into(),
            tools: Vec::new(),
            connected: false,
        }];
        let body = build(&ctx_with(&integrations, &[])).unwrap();
        assert!(!body.contains("## Connected Integrations"));
    }
}
