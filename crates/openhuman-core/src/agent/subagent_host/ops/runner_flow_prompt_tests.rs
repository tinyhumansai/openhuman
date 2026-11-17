use super::*;

#[cfg(feature = "flows")]
fn prompt_context<'a>(workspace: &'a std::path::Path, agent_id: &'a str) -> PromptContext<'a> {
    static VISIBLE: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    PromptContext {
        workspace_dir: workspace,
        model_name: "test",
        agent_id,
        flow_archetype_text: None,
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: VISIBLE.get_or_init(std::collections::HashSet::new),
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn typed_subagent_prompts_use_scoped_bus_text_for_both_flow_builtins() {
    let temp = tempfile::tempdir().expect("workspace");
    let config = std::sync::Arc::new(crate::config::Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("action"),
        config_path: temp.path().join("config.toml"),
        ..crate::config::Config::default()
    });
    let registry = AgentDefinitionRegistry::builtins_only();

    for (agent_id, expected_tool) in [
        ("workflow_builder", "propose_workflow"),
        ("flow_discovery", "suggest_workflows"),
    ] {
        let definition = registry.get(agent_id).expect("flow builtin");
        let context = prompt_context(temp.path(), agent_id);
        let loaded_config = Ok(std::sync::Arc::clone(&config));
        let body = load_typed_prompt_source(definition, &loaded_config, context)
            .await
            .unwrap_or_else(|error| panic!("loading {agent_id} prompt: {error}"));
        assert!(
            body.contains(expected_tool),
            "{agent_id} prompt missing its tool"
        );
        assert!(body.contains("## Workspace"));
    }
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn typed_flow_builtin_requires_its_loaded_config() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let definition = registry
        .get("workflow_builder")
        .expect("workflow_builder builtin");
    let temp = tempfile::tempdir().expect("workspace");
    let context = prompt_context(temp.path(), "workflow_builder");
    let loaded_config = Err("config unavailable".into());
    let error = load_typed_prompt_source(definition, &loaded_config, context)
        .await
        .expect_err("bus-backed prompt must not use ambient config");
    assert!(error.to_string().contains("scoped runtime config"));
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn typed_static_prompt_override_skips_tinyflows_even_when_disabled() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let mut definition = registry
        .get("workflow_builder")
        .expect("workflow_builder builtin")
        .clone();
    definition.system_prompt = PromptSource::Inline("session-owned prompt".into());
    let mut config = crate::config::Config::default();
    config.components.tinyflows = false;
    let config = std::sync::Arc::new(config);
    let temp = tempfile::tempdir().expect("workspace");
    let context = prompt_context(temp.path(), "workflow_builder");

    let loaded_config = Ok(config);
    let body = load_typed_prompt_source(&definition, &loaded_config, context)
        .await
        .expect("static prompt override does not access the module");
    assert_eq!(body, "session-owned prompt");
}
