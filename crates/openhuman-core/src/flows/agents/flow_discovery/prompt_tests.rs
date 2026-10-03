use super::*;
use crate::agent::prompts::ToolCallFormat;
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
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
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
