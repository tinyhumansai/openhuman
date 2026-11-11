use crate::agent::prompts::{PromptContext, PromptTool};

#[test]
fn builder_prompt_composes_supplied_archetype_tools_and_workspace() {
    let temp = tempfile::tempdir().expect("workspace");
    let tools = [PromptTool::new(
        "save_workflow",
        "Save the proposed workflow",
    )];
    let visible = std::collections::HashSet::from(["save_workflow".to_string()]);
    let context = PromptContext {
        workspace_dir: temp.path(),
        model_name: "test",
        agent_id: "workflow_builder",
        flow_archetype_text: Some("module-owned builder archetype"),
        tools: &tools,
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &visible,
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };

    let prompt = super::prompt::build(&context).expect("render builder prompt");
    assert!(prompt.starts_with("module-owned builder archetype"));
    assert!(prompt.contains("save_workflow"));
    assert!(prompt.contains("## Workspace"));
    assert!(prompt.contains("## Passing graphs to tools"));
}
