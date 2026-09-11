use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, PromptContext, ToolCallFormat};
use std::collections::HashSet;

#[test]
fn prompt_section_renders_expected_text() {
    let section = AgentProfilePromptSection::new("  Be concise.  ".into());
    assert_eq!(section.name(), "agent_profile");
    let visible_tool_names = HashSet::new();
    let ctx = PromptContext {
        workspace_dir: std::path::Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "orchestrator",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &visible_tool_names,
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
    let rendered = section.build(&ctx).expect("render profile section");
    assert!(rendered.starts_with("## Agent profile"));
    assert!(rendered.contains("Be concise."));

    let empty = AgentProfilePromptSection::new("   ".into());
    assert_eq!(empty.build(&ctx).expect("empty profile section"), "");
}

fn empty_ctx<'a>(visible: &'a HashSet<String>) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: std::path::Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "orchestrator",
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
fn cross_profile_workspace_notice_names_path_and_boundary() {
    let notice =
        cross_profile_workspace_notice("alice", std::path::Path::new("/act/profiles/alice"));
    assert!(notice.contains("alice"));
    assert!(notice.contains("/act/profiles/alice"));
    assert!(notice.contains("off-limits"));
}

#[test]
fn workspace_notice_renders_with_and_without_body() {
    let visible = HashSet::new();
    let ctx = empty_ctx(&visible);

    // Notice-only (no persona body) still emits the block + the notice.
    let notice_only = AgentProfilePromptSection::new(String::new())
        .with_workspace_notice("boundary sentence.".into());
    let rendered = notice_only.build(&ctx).expect("render notice-only");
    assert!(rendered.starts_with("## Agent profile"));
    assert!(rendered.contains("boundary sentence."));

    // Body + notice: both present, body first.
    let both = AgentProfilePromptSection::new("Be terse.".into())
        .with_workspace_notice("boundary sentence.".into());
    let rendered = both.build(&ctx).expect("render both");
    let body_at = rendered.find("Be terse.").expect("body present");
    let notice_at = rendered.find("boundary sentence.").expect("notice present");
    assert!(body_at < notice_at, "persona body must precede the notice");

    // A blank notice is dropped, leaving only the body.
    let blank_notice =
        AgentProfilePromptSection::new("Be terse.".into()).with_workspace_notice("  ".into());
    let rendered = blank_notice.build(&ctx).expect("render blank notice");
    assert!(rendered.contains("Be terse."));
    assert!(!rendered.contains("off-limits"));
}
