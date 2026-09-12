//! System prompt builder for the `integrations_agent` built-in agent.
//!
//! `integrations_agent` is the one sub-agent that executes Composio actions
//! directly — every other agent delegates to it via `spawn_subagent`. It is
//! composio-only: it drives a single Composio toolkit per spawn.
//!
//! That means the prompt owns one block nobody else renders:
//!
//! * `## Connected Integrations` — the list of Composio toolkits the
//!   user has connected, framed as "you have direct access to the
//!   action tools in your tool list" rather than "delegate to integrations_agent".
//!
//! (It used to also render an `## Available Skills` workflow catalogue, but
//! workflow discovery + execution moved to the orchestrator with the
//! skills→workflows unification — `list_workflows` / `run_workflow`. The
//! integrations_agent has no run_workflow tool, so advertising that catalogue
//! here only promised capabilities it couldn't use.)
//!
//! This block lives here (not in the shared prompts module) so the delegator
//! agents stay lean and the integrations_agent-specific wording isn't a branch
//! on `agent_id` somewhere else.

use crate::openhuman::agent::context::prompt::{
    render_safety, render_tools, render_user_files, render_workspace, ConnectedIntegration,
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
        if ci.connections.len() > 1 {
            let _ = writeln!(
                out,
                "- **{}** ({} accounts) — {}",
                ci.toolkit,
                ci.connections.len(),
                ci.description
            );
            for conn in &ci.connections {
                let label = conn.label.as_deref().unwrap_or("(unlabeled)");
                let default_marker = if conn.is_default { " [default]" } else { "" };
                let _ = writeln!(
                    out,
                    "  - `connection_id: \"{}\"` — {}{}",
                    conn.connection_id, label, default_marker
                );
            }
        } else {
            let _ = writeln!(out, "- **{}** — {}", ci.toolkit, ci.description);
        }
    }

    // Surface pref-gated tools so the agent can honestly say "I have this
    // capability but it needs the {scope} toggle in Connections → {toolkit}".
    // The agent CANNOT call these directly (no parameters schema is exposed)
    // and CANNOT flip the gating scope itself — there is no agent-callable
    // scope-elevate tool. The user must toggle the scope in the Connections
    // UI; after the next prompt rebuild the action graduates into the
    // callable list above. The per-row `unlock paths` rendered below carry
    // the exact UI hint the agent should show.
    let mut has_gated = false;
    let mut connected_with_gated = 0usize;
    for ci in integrations.iter().filter(|ci| ci.connected) {
        if !ci.gated_tools.is_empty() {
            has_gated = true;
            connected_with_gated += 1;
        }
    }
    tracing::debug!(
        total_integrations = integrations.len(),
        has_gated,
        connected_with_gated,
        "[integrations-prompt] gated-tools scan complete"
    );
    if has_gated {
        out.push_str(
            "\n### Additional capabilities behind a permission toggle\n\n\
             These actions exist in the toolkit but are NOT currently in your callable \
             tool list — the user has not granted the required scope. Do NOT pretend \
             they're unavailable. When the user asks for one (or you'd otherwise need \
             it), tell them what the action does and present ALL of its `unlock paths` \
             listed below so the user can choose how to enable it. Never drop a path or \
             rewrite it into your own framing.\n\n",
        );
        for ci in integrations
            .iter()
            .filter(|ci| ci.connected && !ci.gated_tools.is_empty())
        {
            let _ = writeln!(out, "- **{}**:", ci.toolkit);
            for gt in &ci.gated_tools {
                let desc = if gt.description.is_empty() {
                    "(no description)".to_string()
                } else {
                    gt.description.clone()
                };
                let _ = writeln!(
                    out,
                    "  - `{}` — {} (requires `{}` scope)",
                    gt.name, desc, gt.required_scope
                );
                for path in &gt.unlock_paths {
                    let _ = writeln!(out, "    - unlock path: {path}");
                }
            }
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
