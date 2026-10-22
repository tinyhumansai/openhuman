/// The registry override must survive the real session-host parent snapshot,
/// not only the builder's schema assertions. This drives the same tool
/// execution path a running turn uses and verifies that an enabled custom
/// child passes both definition lookup and the parent's effective allowlist
/// (#6934).
#[tokio::test]
async fn running_session_allows_a_registry_override_custom_spawn() {
    use crate::agent::orchestration::tools::SpawnAsyncSubagentTool;
    use crate::agent_registry::host_definition::AgentDefinitionRegistry;
    use crate::agent_registry::types::AgentSubagentPolicy;
    use crate::agent_registry::{AgentRegistryEntry, AgentRegistrySource};
    use crate::config::Config;

    let entry = |id: &str, source, allowlist: Vec<String>| AgentRegistryEntry {
        id: id.into(),
        name: id.into(),
        description: format!("{id} description"),
        source,
        enabled: true,
        model: None,
        system_prompt: Some("Do the work.".into()),
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        tool_rules: None,
        subagents: AgentSubagentPolicy::from_allowlist(allowlist),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    };

    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tmp = tempfile::tempdir().expect("workspace");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace directory");
    config.agent_registry.entries = vec![
        entry("researcher", AgentRegistrySource::Custom, Vec::new()),
        entry(
            "orchestrator",
            AgentRegistrySource::Default,
            vec!["researcher".into()],
        ),
    ];

    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "orchestrator",
    )
    .expect("orchestrator session");
    host.ensure_runtime_session().expect("warm runtime");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("turn prelude");
    let parent = prelude.parent_context(None);
    assert!(parent.allowed_subagent_ids.contains("researcher"));
    assert!(parent
        .all_tools
        .iter()
        .any(|tool| tool.name() == "spawn_async_subagent"));

    let result = SpawnAsyncSubagentTool::new()
        .execute_under_parent(
            serde_json::json!({
                "agent_id": "researcher",
                "prompt": "survey the literature",
            }),
            tinytools::ToolCallOptions::default(),
            None,
            parent,
        )
        .await
        .expect("tool execution");
    let output = result.output();
    assert!(!output.contains("unknown agent_id"), "{output}");
    assert!(!output.contains("subagents.allowlist"), "{output}");
    assert!(output.contains("no parent chat thread"), "{output}");
}
