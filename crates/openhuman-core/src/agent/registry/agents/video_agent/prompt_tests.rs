use super::*;
use crate::agent::prompts::ToolCallFormat;
use std::collections::HashSet;

#[test]
fn build_returns_nonempty_body() {
    let visible: HashSet<String> = HashSet::new();
    let ctx = PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "vision-v1",
        agent_id: "video_agent",
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
    assert!(body.contains("Video-generation specialist"));
}
