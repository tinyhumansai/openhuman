use super::*;

#[cfg(feature = "flows")]
#[tokio::test]
async fn debug_dump_composes_each_flow_prompt_from_the_copilot_bus() {
    let temp = tempfile::tempdir().expect("workspace");
    let mut config = Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("action"),
        config_path: temp.path().join("config.toml"),
        ..Config::default()
    };
    config.default_model = Some("test/session-fixture".to_string());
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("initialize builtin registry");
    let ctx = crate::core::runtime::DispatchCtx::host_default();

    for (agent_id, required) in [
        ("workflow_builder", "propose_workflow"),
        ("flow_discovery", "suggest_workflows"),
    ] {
        let prompt = render_via_session(&ctx, &config, agent_id)
            .await
            .unwrap_or_else(|error| panic!("rendering {agent_id}: {error:#}"));
        assert!(
            prompt.text.contains(required),
            "{agent_id} prompt missing {required}"
        );
        assert!(prompt.text.contains("## Workspace"));
    }
}
