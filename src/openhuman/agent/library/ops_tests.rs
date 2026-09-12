use super::*;
use crate::openhuman::agent::harness::definition::{PromptSource, SkillsWildcard};

fn definition() -> AgentDefinition {
    AgentDefinition {
        id: "researcher".to_string(),
        when_to_use: "Use for research.".to_string(),
        display_name: Some("Researcher".to_string()),
        system_prompt: PromptSource::Inline("hidden prompt".to_string()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: false,
        omit_memory_md: true,
        model: ModelSpec::Hint("reasoning".to_string()),
        temperature: 0.2,
        tools: ToolScope::Named(vec!["web_search".to_string(), "file_read".to_string()]),
        disallowed_tools: vec!["file_read".to_string()],
        skill_filter: None,
        extra_tools: vec!["memory_search".to_string(), "web_search".to_string()],
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::ReadOnly,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![
            SubagentEntry::AgentId("critic".to_string()),
            SubagentEntry::Skills(SkillsWildcard {
                skills: "*".to_string(),
            }),
        ],
        delegate_name: None,
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

#[test]
fn metadata_projection_omits_prompt_and_paths() {
    let display = metadata_from_definition(&definition());

    assert_eq!(display.id, "researcher");
    assert_eq!(display.display_name, "Researcher");
    assert_eq!(display.model.kind, "hint");
    assert_eq!(display.model.value.as_deref(), Some("reasoning"));
    match &display.tools {
        ToolScope::Named(names) => {
            assert_eq!(
                names,
                &vec!["web_search".to_string(), "file_read".to_string()]
            );
        }
        ToolScope::Wildcard => panic!("expected named tool scope"),
    }
    assert_eq!(
        display.direct_tool_names,
        vec!["memory_search", "web_search"]
    );
    assert_eq!(display.direct_tool_count, 2);
    assert!(!display.uses_wildcard_tools);
    assert_eq!(display.subagent_ids, vec!["critic"]);
    assert!(display.includes_profile);
    assert!(!display.includes_memory_md);
    assert!(!display.includes_memory_context);
    assert!(display.can_run_as_user_facing_worker);
    assert!(!display.write_capable);

    let json = serde_json::to_value(display).expect("serialize display");
    assert!(json.get("system_prompt").is_none());
    assert!(json.get("prompt").is_none());
}
