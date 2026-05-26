//! System prompt builder for the `orchestrator` built-in agent.
//!
//! The orchestrator follows a direct-first policy: respond directly or use
//! cheap direct tools whenever possible, and delegate only for specialised
//! execution. It never executes Composio actions itself; the integration
//! block points to the single collapsed `delegate_to_integrations_agent`
//! tool (synthesised by `orchestrator_tools::collect_orchestrator_tools`,
//! #1335) for true external-service operations, with the toolkit slug
//! passed as an argument. That prose lives here (not in the shared
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
         IMPORTANT: You MUST use the `delegate_to_integrations_agent` tool for any request \
         involving connected services. You do NOT have direct access to these services — all \
         interaction must go through delegation. Never claim you cannot access a connected \
         service without first attempting delegation.\n\n\
         The following services have an active connection. Their tool implementations \
         live inside the `integrations_agent` sub-agent — NOT in your own tool list. \
         Delegate with `delegate_to_integrations_agent`, passing the toolkit slug as \
         `toolkit`:\n\n",
    );
    for ci in connected {
        // Use the same slug canonicalisation as `collect_orchestrator_tools`
        // so the `toolkit` arg the orchestrator emits always matches the
        // enum the synthesised tool accepts.
        let slug = sanitise_slug(&ci.toolkit);
        let _ = writeln!(
            out,
            "- **{}** (`toolkit: \"{}\"`): {}",
            ci.toolkit, slug, ci.description
        );
    }
    // CRITICAL behavioural rule. Without this, the orchestrator answers
    // "can you do X with {toolkit}?" from its training-data priors about
    // "what gmail/notion/slack usually does", which is consistently a
    // SUBSET of the real per-toolkit catalogue (no bulk-delete, no
    // batch-modify, no admin/destructive actions, etc.). The result is a
    // confident wrong refusal ("nope, I can't delete emails") even when
    // the action is in the actual tool list. The `integrations_agent`
    // has the ground-truth tool catalogue (`tools` + `gated_tools`); only
    // it can answer "can I do X?" honestly. Force-delegate capability
    // questions, not just task requests.
    // The cross-chat bullet names the canonical header literal verbatim
    // so the model knows exactly which block to mistrust. Sourced from
    // CROSS_CHAT_HEADER (single source of truth) — drift would silently
    // detune the rule.
    let cross_chat_header_for_prompt =
        crate::openhuman::agent::memory_loader::CROSS_CHAT_HEADER.trim_end();
    let _ = write!(
        out,
        "\n### Capability questions about connected toolkits\n\n\
         Your prior knowledge of \"what a toolkit can do\" is UNRELIABLE — the \
         real per-toolkit catalogue is wider than the common-knowledge summary \
         (e.g. Gmail exposes bulk delete, batch modify, thread trash, etc.) and \
         the user may have enabled scopes that expose further destructive actions. \
         Therefore:\n\n\
         - If the user asks **\"can you do X with {{toolkit}}?\"** or \"does \
         {{toolkit}} support Y?\" for a connected toolkit above, **DO NOT** answer \
         from priors. **DELEGATE** to `integrations_agent` first and let it \
         inspect its live tool list (including `gated_tools` behind permission \
         toggles) before answering.\n\
         - If the user requests an **action** on a connected toolkit (delete, \
         move, send, modify, label, etc.), **DELEGATE immediately**. Do not \
         pre-emptively refuse with \"I can't do that\" — that's a confabulation \
         unless `integrations_agent` itself has already reported the action as \
         unavailable.\n\
         - The only honest \"no\" comes back from a delegation that found the \
         action neither in the visible `tools` list nor in the `gated_tools` \
         (permission-toggle) list of the sub-agent.\n\
         - **Cross-chat context is historical, not authoritative.** If the \
         `{cross_chat_header_for_prompt}` block contains a past \"I can / can't \
         do X with {{toolkit}}\" statement, treat it as a snapshot from an \
         earlier moment. The tool list, connected integrations, and per-toolkit \
         scope toggles (read / write / admin) can all change between chats — a \
         past refusal may be stale. Verify against the **current** `## Connected \
         Integrations` block above and (when in doubt) **DELEGATE** before \
         quoting any past capability claim. Never echo a stale \"I can't\" \
         without re-checking.\n\n",
    );
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
            "Default bias: **do not spawn a sub-agent when a direct response or direct tool call is sufficient**"
        ));
        // Step 2 of the decision tree now explicitly routes live external-service
        // requests to `delegate_to_integrations_agent` rather than `memory_tree`.
        assert!(body.contains("Does the request name (or imply) a connected external service?"));
        assert!(body.contains("Do this even if `memory_tree` could plausibly answer"));
    }

    #[test]
    fn build_routes_live_facts_to_research_tool() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(body.contains("use `research`"));
        assert!(body.contains("weather, forecasts, current temperatures"));
        assert!(body.contains("\"use Grok/web/live data\""));
        assert!(body.contains("Do **not** stop at \"on it\""));
        assert!(
            !body.contains("delegate_researcher"),
            "orchestrator prompt should name the synthesized researcher tool"
        );
    }

    #[test]
    fn build_emits_delegation_guide_with_collapsed_tool() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email access.".into(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            non_active_status: None,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("delegate_to_integrations_agent"));
        assert!(body.contains("toolkit: \"gmail\""));
        // Must NOT contain the old per-toolkit fan-out tool names.
        assert!(!body.contains("delegate_gmail"));
        // Must NOT contain the old verbose spawn_subagent snippet.
        assert!(!body.contains("spawn_subagent(agent_id=\"integrations_agent\""));
        // Delegator voice must NOT use the skill-executor wording.
        assert!(!body.contains("You have direct access"));
        // Must contain the hardened delegation instruction.
        assert!(
            body.contains("IMPORTANT"),
            "delegation guide must contain the IMPORTANT instruction"
        );
        assert!(
            body.contains("Never claim you cannot access a connected service without first attempting delegation"),
            "delegation guide must instruct the model to always attempt delegation"
        );
    }

    #[test]
    fn build_does_not_route_scope_errors_as_disconnected() {
        let body = build(&ctx_with(&[])).unwrap();
        assert!(body.contains("[composio:error:insufficient_scope]"));
        assert!(body.contains("missing required permissions"));
        assert!(body.contains("connection exists but needs additional permissions"));
        assert!(body.contains("Settings"));
        assert!(body.contains("Connections"));
    }

    #[test]
    fn delegation_guide_uses_compact_collapsed_format() {
        let integrations = vec![ConnectedIntegration {
            toolkit: "gmail".into(),
            description: "Email access.".into(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            non_active_status: None,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(body.contains("## Connected Integrations"));
        assert!(body.contains("delegate_to_integrations_agent"));
        // Old verbose / per-toolkit forms must be gone.
        assert!(!body.contains("delegate_gmail"));
        assert!(!body.contains("spawn_subagent(agent_id=\"integrations_agent\""));
    }

    #[test]
    fn build_hides_unconnected_integrations() {
        // Only connected toolkits make it into the Delegation Guide
        // — unconnected entries would just trigger a downstream
        // pre-flight rejection, so keeping them out keeps the prompt
        // focused on what the orchestrator can actually delegate.
        let integrations = vec![
            ConnectedIntegration {
                toolkit: "gmail".into(),
                description: "Email.".into(),
                tools: Vec::new(),
                gated_tools: Vec::new(),
                connected: true,
                non_active_status: None,
            },
            ConnectedIntegration {
                toolkit: "linear".into(),
                description: "Tracker.".into(),
                tools: Vec::new(),
                gated_tools: Vec::new(),
                connected: false,
                non_active_status: None,
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
            gated_tools: Vec::new(),
            connected: false,
            non_active_status: None,
        }];
        let body = build(&ctx_with(&integrations)).unwrap();
        assert!(!body.contains("## Connected Integrations"));
    }
}
