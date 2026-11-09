use super::*;
use tinyagents_definition::{
    DefinitionSource, ModelSpec, PromptSource, SandboxMode, SkillsWildcard, ToolScope,
};

fn orchestrator() -> AgentDefinition {
    AgentDefinition {
        id: "orchestrator".into(),
        when_to_use: String::new(),
        display_name: None,
        system_prompt: PromptSource::Inline(String::new()),
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        tool_rules: None,
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![
            SubagentEntry::AgentId("image_agent".into()),
            SubagentEntry::AgentId("research_agent".into()),
            SubagentEntry::Skills(SkillsWildcard { skills: "*".into() }),
        ],
        delegate_name: None,
        agent_tier: tinyagents_definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        extensions: crate::agent_registry::host_definition::AgentExt {
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression::Auto,
            searches_connected_mcp: false,
            ..Default::default()
        },
        role: None,
    }
}

#[test]
fn agent_id_allowlist_entries_do_not_synthesize_model_facing_dispatch_tools() {
    let tools = collect_orchestrator_tools(&orchestrator(), &[]);

    assert!(tools.is_empty());
    let spawn = crate::agent::registration::dialogue_tools(
        std::path::Path::new("."),
        &crate::config::Config::default(),
    );
    let subagent_tools: Vec<_> = spawn
        .iter()
        .filter(|tool| tool.name().contains("subagent") || tool.name().starts_with("delegate_"))
        .map(|tool| tool.name())
        .collect();
    assert_eq!(subagent_tools, ["spawn_subagent"]);
}
