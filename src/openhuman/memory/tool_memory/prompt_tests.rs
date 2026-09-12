use super::*;
use crate::openhuman::agent::prompts::types::{LearnedContextData, PromptContext, ToolCallFormat};
use crate::openhuman::memory::tool_memory::{ToolMemoryPriority, ToolMemoryRule, ToolMemorySource};

fn rule(tool: &str, body: &str, priority: ToolMemoryPriority) -> ToolMemoryRule {
    ToolMemoryRule {
        id: format!("{tool}/{body}"),
        tool_name: tool.into(),
        rule: body.into(),
        priority,
        source: ToolMemorySource::UserExplicit,
        tags: vec![],
        created_at: "2026-05-11T00:00:00Z".into(),
        updated_at: "2026-05-11T00:00:00Z".into(),
    }
}

#[test]
fn section_empty_returns_blank_build_output() {
    let section = ToolMemoryRulesSection::empty();
    assert!(section.is_empty());
}

#[test]
fn section_renders_via_prompt_section_trait() {
    // Exercise the host PromptSection glue over the crate section: build()
    // returns the at-construction snapshot regardless of PromptContext.
    let section = ToolMemoryRulesSection::new(vec![rule(
        "email",
        "never email Sarah",
        ToolMemoryPriority::Critical,
    )]);
    assert!(!section.is_empty());
    let visible = std::collections::HashSet::new();
    let ctx = PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "test",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &visible,
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
    };
    let built = section.build(&ctx).unwrap();
    assert!(built.contains("never email Sarah"));
}
