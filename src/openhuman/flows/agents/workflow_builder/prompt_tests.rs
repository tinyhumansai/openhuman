use super::*;
use crate::openhuman::agent::context::prompt::{LearnedContextData, ToolCallFormat};
use std::collections::HashSet;

fn ctx() -> PromptContext<'static> {
    static VISIBLE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    let visible = VISIBLE.get_or_init(HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new("."),
        model_name: "test",
        agent_id: "workflow_builder",
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

// A test asserting `body.to_lowercase().contains("propose")` used to live
// here. It was near-tautological: "propose"/"proposal"/"propose_workflow"
// appear dozens of times throughout the archetype regardless of whether
// the propose-only invariant is actually taught correctly, so the
// assertion could not fail in any way that mattered (P-m5). The real
// invariant — every authoring turn is propose-only, persistence gated
// behind the user's explicit ask — is pinned with specific, falsifiable
// substrings in `builder_prompt.rs`: `create_prompt_frames_propose_only`,
// `build_is_propose_only_and_injects_flow_id_as_context` (the #4596
// regression guard), and the `standing_prompt_*` tests that assert exact
// phrases like "Do NOT save_workflow" and "Do not save, enable, or run
// anything". Removed rather than strengthened to avoid duplicating that
// coverage here.
