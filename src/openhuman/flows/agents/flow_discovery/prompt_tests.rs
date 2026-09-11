use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn ctx() -> PromptContext<'static> {
    static VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    let visible = VISIBLE.get_or_init(HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "flow_discovery",
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
    let body = build(&ctx()).unwrap();
    assert!(!body.is_empty());
}

#[test]
fn prompt_teaches_the_read_only_emit_invariant() {
    let body = build(&ctx()).unwrap();
    let lc = body.to_lowercase();
    assert!(lc.contains("suggest_workflows"), "must name the emit tool");
    assert!(
        lc.contains("read-only") || lc.contains("never act") || lc.contains("never build"),
        "prompt must teach the read-only invariant"
    );
}
