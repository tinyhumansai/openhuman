use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn empty_ctx() -> PromptContext<'static> {
    static EMPTY_VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "mcp_setup",
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
    assert!(body.contains("MCP Setup Agent"));
}

#[test]
fn archetype_documents_opaque_ref_invariant() {
    let body = build(&empty_ctx()).unwrap();
    assert!(body.contains("never enters your context"));
    assert!(body.contains("secret://"));
}

#[test]
fn archetype_documents_standard_flow_steps() {
    let body = build(&empty_ctx()).unwrap();
    for needle in [
        "mcp_setup_search",
        "mcp_setup_get",
        "mcp_setup_request_secret",
        "mcp_setup_test_connection",
        "mcp_setup_install_and_connect",
        "ask_user_clarification",
    ] {
        assert!(body.contains(needle), "prompt missing `{needle}`");
    }
}
