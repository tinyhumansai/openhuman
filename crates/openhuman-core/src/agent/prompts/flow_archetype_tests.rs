use super::*;
use tinyagents_definition::DefinitionSource;

#[test]
fn only_dynamic_builtin_flow_definitions_use_the_bus_prompt() {
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    for id in ["workflow_builder", "flow_discovery"] {
        let mut definition = registry.get(id).expect("builtin flow agent").clone();
        assert!(is_bus_backed_builtin(&definition));

        definition.system_prompt = PromptSource::Inline("session prompt override".into());
        assert!(!is_bus_backed_builtin(&definition));

        let mut definition = registry.get(id).expect("builtin flow agent").clone();
        definition.source = DefinitionSource::CustomRegistry;
        assert!(is_bus_backed_builtin(&definition));

        definition.extensions.tinyflows_prompt_owner = None;
        assert!(!is_bus_backed_builtin(&definition));
    }
}

#[test]
fn renamed_dynamic_flow_builtin_keeps_its_prompt_owner() {
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let mut definition = registry
        .get("workflow_builder")
        .expect("workflow_builder builtin")
        .clone();
    definition.id = "library_workflow_builder".into();

    assert!(
        is_bus_backed_builtin(&definition),
        "a library actor id must not erase the resolved builtin prompt owner"
    );
    assert_eq!(prompt_owner(&definition), Some("workflow_builder"));
}

#[tokio::test]
async fn flow_prompt_requires_the_scoped_config_and_respects_component_disable() {
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = registry
        .get("workflow_builder")
        .expect("workflow_builder builtin");
    let error = load_for_definition(None, definition)
        .await
        .expect_err("scoped config is required");
    assert!(error.to_string().contains("scoped runtime config"));

    let mut config = Config::default();
    config.components.tinyflows = false;
    let error = load_for_definition(Some(&config), definition)
        .await
        .expect_err("disabled module must fail closed");
    assert!(format!("{error:#}").contains("COMPONENT_DISABLED:tinyflows"));
}

#[tokio::test]
async fn flow_prompt_fetch_uses_the_copilot_get_prompt_bus_member() {
    let registry = crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let mut config = Config::default();
    let temp = tempfile::tempdir().expect("workspace");
    config.workspace_dir = temp.path().join("workspace");
    config.config_path = temp.path().join("config.toml");

    let mut texts = std::collections::HashMap::new();
    for id in ["workflow_builder", "flow_discovery"] {
        let definition = registry.get(id).expect("builtin flow agent");
        let text = load_for_definition(Some(&config), definition)
            .await
            .expect("bus prompt fetch")
            .expect("flow builtin prompt");
        assert!(!text.trim().is_empty(), "{id} prompt must be nonempty");

        let mut renamed = definition.clone();
        renamed.id = format!("library_{id}");
        let renamed_text = load_for_definition(Some(&config), &renamed)
            .await
            .expect("renamed builtin fetches its original prompt owner")
            .expect("renamed flow builtin prompt");
        assert_eq!(renamed_text, text, "actor id must not become GetPrompt key");

        texts.insert(id, text);
    }
    assert_ne!(texts["workflow_builder"], texts["flow_discovery"]);
}
