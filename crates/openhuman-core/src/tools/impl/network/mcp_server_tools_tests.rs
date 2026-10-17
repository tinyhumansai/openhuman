use super::*;
use serde_json::json;

fn presentation(server_id: &str, deferred: bool) -> tinymcp_bus::McpToolPresentation {
    tinymcp_bus::McpToolPresentation {
        name: "mcp_weather_forecast".into(),
        legacy_name: "mcp_forecast_legacy".into(),
        server_id: server_id.into(),
        remote_name: "forecast".into(),
        family: "weather".into(),
        description: "Read the forecast".into(),
        parameters: json!({"type":"object","properties":{"city":{"type":"string"}}}),
        deferred,
        tags: vec!["server:weather".into(), "tool:forecast".into()],
    }
}

fn config() -> Config {
    let mut config = Config::default();
    config
        .mcp_client
        .servers
        .push(crate::config::McpServerConfig {
            server: tinymcp_bus::McpServerConfig {
                name: "weather".into(),
                endpoint: "https://example.test/mcp".into(),
                ..Default::default()
            },
            ..Default::default()
        });
    config
}

#[test]
fn module_prepared_static_tool_keeps_its_contract_and_exposure() {
    let config = config();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let mut tools = configured_server_tools_from_presentations(
        &config,
        vec![presentation("weather", true)],
        &security,
    );
    assert_eq!(tools.len(), 1);
    let tool = tools.pop().expect("prepared tool");
    assert_eq!(tool.name(), "mcp_weather_forecast");
    assert_eq!(tool.description(), "Read the forecast");
    assert_eq!(
        tool.parameters_schema()["properties"]["city"]["type"],
        "string"
    );
    assert_eq!(tool.exposure(), ToolExposure::Deferred);
    assert_eq!(tool.family(), Some("weather"));
}

#[test]
fn module_presentation_for_an_unconfigured_server_is_not_executable() {
    let config = config();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    assert!(configured_server_tools_from_presentations(
        &config,
        vec![presentation("removed-server", false)],
        &security,
    )
    .is_empty());
}
