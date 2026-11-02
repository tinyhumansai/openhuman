use super::*;

#[cfg(feature = "flows")]
#[tokio::test]
async fn cold_flow_prompt_uses_the_module_and_warm_resume_does_not_refetch() {
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let temp = tempfile::tempdir().expect("workspace");
    let config = crate::config::Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("action"),
        config_path: temp.path().join("config.toml"),
        ..Default::default()
    };
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "flow_discovery",
    )
    .await
    .expect("flow_discovery session");
    host.ensure_runtime_session().expect("runtime session");
    let mut prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");

    let cold = prelude.prepare(true).await.expect("cold flow prompt");
    let prompt = cold
        .prefix
        .expect("cold preparation has a prompt prefix")
        .messages()
        .iter()
        .map(tinyinference_llm::message::Message::text)
        .collect::<String>();
    assert!(prompt.contains("suggest_workflows"));
    assert!(prompt.contains("read-only") || prompt.contains("never act"));

    let mut disabled_config = (*prelude.runtime_config.as_deref().expect("scoped config")).clone();
    disabled_config.components.tinyflows = false;
    prelude.runtime_config = Some(Arc::new(disabled_config));
    let warm = prelude
        .prepare(false)
        .await
        .expect("warm/resumed prompt does not refetch TinyFlows text");
    assert!(warm.prefix.is_none());

    let error = match prelude.prepare(true).await {
        Ok(_) => panic!("cold flow prompt must honor the disabled component"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("COMPONENT_DISABLED:tinyflows"));
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn explicit_flow_session_prompt_override_does_not_require_tinyflows_or_config() {
    use tinyagents_definition::{DefinitionSource, PromptSource};

    let temp = tempfile::tempdir().expect("workspace");
    let config = crate::config::Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("action"),
        config_path: temp.path().join("config.toml"),
        ..Default::default()
    };
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    let mut definition =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
            .get("workflow_builder")
            .expect("workflow_builder builtin")
            .clone();
    definition.source = DefinitionSource::CustomRegistry;
    definition.system_prompt = PromptSource::Inline("session-owned flow prompt".into());

    let mut host = crate::agent::OpenHumanSessionHost::from_config_with_definition(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &definition,
    )
    .await
    .expect("explicit flow definition session");
    host.ensure_runtime_session().expect("runtime session");
    let mut prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");
    prelude.runtime_config = None;

    let prepared = prelude
        .prepare(true)
        .await
        .expect("static session override does not load TinyFlows");
    let prompt = prepared
        .prefix
        .expect("cold preparation has a prompt prefix")
        .messages()
        .iter()
        .map(tinyinference_llm::message::Message::text)
        .collect::<String>();
    assert!(prompt.contains("session-owned flow prompt"));
}
