use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn test_ctx() -> PromptContext<'static> {
    // Leak a HashSet so the &reference satisfies the 'static-ish lifetime
    // the helper needs in this throwaway test context.
    let visible: &'static HashSet<String> = Box::leak(Box::new(HashSet::new()));
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "context_scout",
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
    let body = build(&test_ctx()).unwrap();
    assert!(!body.is_empty());
}

#[test]
fn body_describes_the_context_bundle_contract() {
    let body = build(&test_ctx()).unwrap();
    assert!(body.contains("[context_bundle]"));
    assert!(body.contains("has_enough_context"));
    assert!(body.contains("recommended_tool_calls"));
}

#[test]
fn body_instructs_transcript_and_skill_gathering() {
    // The enrichment is only real if the role prompt actually tells the
    // scout to search past chats and recommend skills — lock that wiring.
    // Past chats are reached through `memory_recall` since the `thread_*`
    // and `transcript_search` tools were removed.
    let body = build(&test_ctx()).unwrap();
    assert!(
        body.contains("memory_recall"),
        "scout prompt must instruct searching past conversations"
    );
    assert!(
        body.contains("recommended_skills"),
        "scout prompt must define the recommended_skills output block"
    );
    assert!(
        body.contains("list_workflows"),
        "scout prompt must point at skill discovery"
    );
}

fn integration(toolkit: &str, connected: bool) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: vec![],
        gated_tools: vec![],
        connected,
        connections: vec![],
        non_active_status: None,
    }
}

#[test]
fn render_connected_integrations_lists_only_connected() {
    let out =
        render_connected_integrations(&[integration("gmail", true), integration("notion", false)]);
    assert!(out.contains("## Connected Integrations"));
    assert!(out.contains("- gmail"));
    assert!(
        !out.contains("notion"),
        "unconnected toolkits must be omitted"
    );
}

#[test]
fn render_connected_integrations_empty_when_none_connected() {
    assert!(render_connected_integrations(&[integration("gmail", false)]).is_empty());
    assert!(render_connected_integrations(&[]).is_empty());
}
