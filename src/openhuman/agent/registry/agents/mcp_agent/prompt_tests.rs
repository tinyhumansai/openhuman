use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn empty_ctx() -> PromptContext<'static> {
    static EMPTY_VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "mcp_agent",
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
        agents_md_global: None,
        agents_md_local: None,
    }
}

#[test]
fn build_returns_nonempty_body() {
    let body = build(&empty_ctx()).unwrap();
    assert!(!body.is_empty());
    assert!(body.contains("MCP Agent"));
}

#[test]
fn archetype_documents_connected_only_invariant() {
    let body = build(&empty_ctx()).unwrap();
    // Must steer away from install (that's mcp_setup's job) and toward
    // the discover → list → call flow over already-connected servers.
    assert!(body.contains("already connected") || body.contains("already-connected"));
    assert!(body.contains("setup_mcp_server"));
}

#[test]
fn archetype_documents_tool_flow() {
    let body = build(&empty_ctx()).unwrap();
    for needle in [
        "mcp_registry_status",
        "mcp_registry_list_tools",
        "mcp_registry_tool_call",
    ] {
        assert!(body.contains(needle), "prompt missing `{needle}`");
    }
}
