use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn ctx_with<'a>(integrations: &'a [ConnectedIntegration]) -> PromptContext<'a> {
    // Leak a HashSet so the returned context borrows a 'static-ish
    // reference — the test owns the value for its lifetime.
    use std::sync::OnceLock;
    static EMPTY_VISIBLE: OnceLock<HashSet<String>> = OnceLock::new();
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "integrations_agent",
        tools: &[],
        workflows: &[],
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
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

#[test]
fn build_returns_nonempty_body() {
    let body = build(&ctx_with(&[])).unwrap();
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
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    }];
    let body = build(&ctx_with(&integrations)).unwrap();
    assert!(body.contains("## Connected Integrations"));
    assert!(body.contains("You have direct access"));
    assert!(body.contains("- **gmail** — Email access."));
    // `integrations_agent` must NOT render the delegator spawn snippet —
    // that belongs on the orchestrator/welcome side.
    assert!(!body.contains("Delegation Guide"));
    assert!(!body.contains("spawn_subagent"));
}

#[test]
fn build_distinguishes_scope_errors_from_disconnected_auth() {
    let body = build(&ctx_with(&[])).unwrap();
    assert!(body.contains("[composio:error:insufficient_scope]"));
    assert!(body.contains("Scope errors are not disconnections"));
    assert!(body.contains("Never say the toolkit is disconnected"));
    assert!(body.contains("Connections → the toolkit"));
    assert!(!body.contains("Settings → Connections"));
    assert!(!body.contains("Settings → Automation & Channels"));
}

#[test]
fn build_skips_unconnected_integrations() {
    let integrations = vec![ConnectedIntegration {
        toolkit: "notion".into(),
        description: "Pages.".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: false,
        connections: Vec::new(),
        non_active_status: None,
    }];
    let body = build(&ctx_with(&integrations)).unwrap();
    assert!(!body.contains("## Connected Integrations"));
}
