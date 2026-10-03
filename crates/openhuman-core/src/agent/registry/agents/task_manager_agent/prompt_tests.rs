use super::*;
use crate::agent::prompts::ToolCallFormat;
use std::collections::HashSet;

#[test]
fn build_returns_task_manager_contract() {
    let visible = HashSet::new();
    let ctx = PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "task_manager_agent",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let body = build(&ctx).unwrap();
    assert!(body.contains("Task Manager Agent"));
    assert!(body.contains("read before you write"));
}
