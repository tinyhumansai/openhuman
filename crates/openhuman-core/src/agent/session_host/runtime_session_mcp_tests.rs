use super::*;

#[cfg(feature = "mcp")]
#[tokio::test]
async fn connected_mcp_actions_enter_search_and_leave_on_disconnect() {
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "orchestrator",
    )
    .await
    .expect("orchestrator");
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");
    prelude
        .mutable
        .lock()
        .expect("prelude state")
        .mcp_tools
        .connected = vec![tinymcp_bus::McpToolPresentation {
        name: "mcp_weather_forecast_abc123".into(),
        legacy_name: "mcp_forecast_legacy".into(),
        server_id: "server-1".into(),
        remote_name: "forecast".into(),
        family: "example/weather".into(),
        description: "Weather forecast".into(),
        parameters: serde_json::json!({"type":"object","properties":{}}),
        deferred: true,
        tags: vec!["mcp.server:example/weather".into()],
    }];

    let action = "mcp_weather_forecast_abc123".to_string();
    prelude.refresh_delegation_tool_surface().unwrap();
    assert!(synthesized_tool_names(&prelude).contains(&action));
    {
        let surface = prelude.tool_surface.lock().expect("tool surface");
        assert!(surface.deferred_tool_names.contains(&action));
        assert!(!surface.visible_tool_names.contains(&action));
    }

    prelude
        .mutable
        .lock()
        .expect("prelude state")
        .mcp_tools
        .connected
        .clear();
    prelude.refresh_delegation_tool_surface().unwrap();
    assert!(!synthesized_tool_names(&prelude).contains(&action));
    assert!(!prelude
        .tool_surface
        .lock()
        .expect("tool surface")
        .deferred_tool_names
        .contains(&action));
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn resumed_mcp_snapshot_survives_a_cold_catalog_and_keeps_its_frozen_spec() {
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;
    let mut host = crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "orchestrator",
    )
    .await
    .expect("orchestrator");
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");

    let name = "mcp_weather_forecast_abc123";
    let mut recorded_spec = spec(name);
    recorded_spec.description = "frozen MCP description".into();
    recorded_spec.parameters = serde_json::json!({"type":"object","frozen":true});
    let recorded = ToolSnapshot::new(vec![recorded_spec.clone()]).expect("recorded snapshot");
    prelude.adopt_recorded_tools(Some(&recorded));
    prelude
        .refresh_delegation_tool_surface()
        .expect("cold refresh");

    let cold = prelude
        .prepare(false)
        .await
        .expect("prepared cold surface")
        .tools
        .expect("cold tool snapshot");
    let (cold_with_recorded, retained) = cold
        .with_retained(&recorded)
        .expect("retain the transcript declaration");
    assert_eq!(retained, 1, "the cold catalog supplied no MCP executor");
    let cold_spec = cold_with_recorded
        .specs()
        .iter()
        .find(|spec| spec.name == name)
        .expect("recorded MCP name survives the cold catalog");
    assert_eq!(
        serde_json::to_value(cold_spec).expect("serialize restored declaration"),
        serde_json::to_value(&recorded_spec).expect("serialize frozen declaration")
    );

    prelude
        .mutable
        .lock()
        .expect("prelude state")
        .mcp_tools
        .connected = vec![tinymcp_bus::McpToolPresentation {
        name: name.into(),
        legacy_name: "mcp_forecast_legacy".into(),
        server_id: "server-1".into(),
        remote_name: "forecast".into(),
        family: "example/weather".into(),
        description: "changed current catalog description".into(),
        parameters: serde_json::json!({"type":"object","current":true}),
        deferred: true,
        tags: vec!["mcp.server_id:server-1".into()],
    }];
    prelude
        .refresh_delegation_tool_surface()
        .expect("refresh from recovered catalog");
    let recovered = prelude
        .prepare(false)
        .await
        .expect("prepared recovered surface")
        .tools
        .expect("recovered tool snapshot");
    let recovered_spec = recovered
        .specs()
        .iter()
        .find(|spec| spec.name == name)
        .expect("recorded MCP name remains present");
    assert_eq!(
        serde_json::to_value(recovered_spec).expect("serialize restored declaration"),
        serde_json::to_value(&recorded_spec).expect("serialize frozen declaration")
    );
}
