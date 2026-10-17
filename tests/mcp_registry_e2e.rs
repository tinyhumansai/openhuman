//! Native-module integration tests for the MCP registry host adapter.
//!
//! These tests use TinyBus' normal loader with a locally supplied TinyMCP
//! artifact. Build one with `node scripts/modules/build.mjs tinymcp` and set
//! `OPENHUMAN_MODULE_SOURCE_DIR` (or put the verified platform library on
//! `OPENHUMAN_MODULE_PATH`) to run them with `--ignored`; no implementation crate is linked
//! into this host test.

#![cfg(feature = "mcp")]

use openhuman_core::config::{schema::ModuleOverride, Config};
use serde_json::{json, Map, Value};

fn fresh_workspace_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifact = openhuman_core::modules::local_artifact("tinymcp")
        .expect("build tinymcp with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR");
    let artifact = artifact
        .canonicalize()
        .expect("TinyMCP artifact path must exist");
    let cfg = Config {
        workspace_dir: tmp.path().to_path_buf(),
        modules: openhuman_core::config::schema::ModulesConfig {
            enabled: true,
            allow_download: false,
            overrides: vec![ModuleOverride::with_path(
                "tinymcp",
                artifact.to_string_lossy().into_owned(),
            )],
            ..Default::default()
        },
        ..Config::default()
    };
    (tmp, cfg)
}

async fn install_server(config: &Config, name: &str, server: Value) -> String {
    let mut servers = Map::new();
    servers.insert(name.to_owned(), server);
    let document = json!({
        "mcpServers": servers
    });
    let applied =
        openhuman_core::mcp::registry::config_ops::mcp_clients_config_set(config, document).await;
    if let Err(error) = &applied {
        let module_status = openhuman_core::modules::ops::list(config)
            .into_iter()
            .find(|module| module.id == "tinymcp");
        panic!("apply mcp.json through the module: {error}; status: {module_status:?}");
    }
    applied.expect("apply mcp.json through the module");
    let status = openhuman_core::mcp::registry::ops::mcp_clients_status(config)
        .await
        .expect("list the module-owned installs");
    status.value["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|entry| entry["qualified_name"] == name))
        .and_then(|entry| entry["server_id"].as_str())
        .map(str::to_owned)
        .expect("applied server appears in module status")
}

fn stdio_server() -> Value {
    json!({
        "command": env!("CARGO_BIN_EXE_test-mcp-stub"),
        "args": [],
        "enabled": false
    })
}

fn http_server(url: &str) -> Value {
    json!({ "url": url, "enabled": false })
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn module_tool_call_reports_unknown_tool_as_error() {
    let (_tmp, config) = fresh_workspace_config();
    let server_id = install_server(&config, "@openhuman-test/echo", stdio_server()).await;
    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(&config, server_id.clone(), true)
        .await
        .expect("enable server");
    openhuman_core::mcp::registry::ops::mcp_clients_connect(&config, server_id.clone())
        .await
        .expect("connect stdio fixture through TinyBus");

    let result = openhuman_core::mcp::registry::ops::mcp_clients_tool_call(
        &config,
        server_id.clone(),
        "does_not_exist".into(),
        json!({}),
    )
    .await
    .expect("tool errors are returned as MCP tool outcomes");
    assert_eq!(result.value["is_error"], true);

    openhuman_core::mcp::registry::ops::mcp_clients_disconnect(&config, server_id)
        .await
        .expect("disconnect fixture");
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn disabling_server_drops_its_live_module_connection() {
    let (_tmp, config) = fresh_workspace_config();
    let server_id = install_server(&config, "@openhuman-test/echo", stdio_server()).await;
    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(&config, server_id.clone(), true)
        .await
        .expect("enable server");
    openhuman_core::mcp::registry::ops::mcp_clients_connect(&config, server_id.clone())
        .await
        .expect("connect stdio fixture through TinyBus");
    assert!(
        openhuman_core::mcp::registry::connections::is_connected_for_config(&config, &server_id)
            .await
    );

    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(&config, server_id.clone(), false)
        .await
        .expect("disable server");
    assert!(
        !openhuman_core::mcp::registry::connections::is_connected_for_config(&config, &server_id)
            .await,
        "disabling an install must close the module-owned connection"
    );
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn disabled_server_accepts_environment_update_without_reconnecting() {
    use std::collections::HashMap;

    let (_tmp, config) = fresh_workspace_config();
    let server_id = install_server(&config, "@openhuman-test/echo", stdio_server()).await;
    let mut env = HashMap::new();
    env.insert("API_KEY".to_owned(), "test-only-secret".to_owned());

    let result =
        openhuman_core::mcp::registry::ops::mcp_clients_update_env(&config, server_id.clone(), env)
            .await
            .expect("confidential disabled-server update succeeds");
    assert_eq!(result.value["status"], "disabled");
    assert_eq!(result.value["env_keys"], json!(["API_KEY"]));
    assert!(
        !openhuman_core::mcp::registry::connections::is_connected_for_config(&config, &server_id)
            .await
    );
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn per_config_lookups_are_isolated_by_workspace() {
    use openhuman_core::mcp::registry::{connections, ops};

    let (_tmp_a, config_a) = fresh_workspace_config();
    let (_tmp_b, config_b) = fresh_workspace_config();
    let server_id = install_server(&config_a, "@openhuman-test/echo", stdio_server()).await;
    ops::mcp_clients_set_enabled(&config_a, server_id.clone(), true)
        .await
        .expect("enable server");
    ops::mcp_clients_connect(&config_a, server_id.clone())
        .await
        .expect("connect server in workspace A");

    assert!(connections::is_connected_for_config(&config_a, &server_id).await);
    assert!(!connections::is_connected_for_config(&config_b, &server_id).await);
    let tools = connections::server_tools_for_config(&config_a, &server_id)
        .await
        .expect("workspace A sees connected tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");

    ops::mcp_clients_disconnect(&config_a, server_id)
        .await
        .expect("disconnect fixture");
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn disabled_install_is_not_dialed_during_startup() {
    use openhuman_core::mcp::registry::{connections, ops};

    let (_tmp, config) = fresh_workspace_config();
    let server_id = install_server(
        &config,
        "@openhuman-test/disabled",
        json!({
            "command": "/nonexistent-disabled-mcp-server",
            "enabled": false
        }),
    )
    .await;
    openhuman_core::mcp::registry::boot::spawn_installed_servers(&config).await;
    let status = ops::mcp_clients_status(&config).await.expect("status");
    let entry = status.value["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|entry| entry["server_id"] == server_id))
        .expect("disabled install remains visible");
    assert_eq!(entry["status"], "disabled");
    assert!(entry["last_error"].is_null());
    assert!(!connections::is_connected_for_config(&config, &server_id).await);
}

#[tokio::test]
#[ignore = "requires a TinyMCP library: node scripts/modules/build.mjs tinymcp"]
async fn per_config_auth_failure_surfaces_401_reason_without_cross_workspace_leak() {
    use openhuman_core::mcp::registry::{connections, ops};
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let upstream = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(401))
        .mount(&upstream)
        .await;

    let (_tmp_a, config_a) = fresh_workspace_config();
    let (_tmp_b, config_b) = fresh_workspace_config();
    let server_id = install_server(
        &config_a,
        "@openhuman-test/remote",
        http_server(&upstream.uri()),
    )
    .await;

    ops::mcp_clients_set_enabled(&config_a, server_id.clone(), true)
        .await
        .expect("enable remote server before exercising its 401 response");

    assert!(
        ops::mcp_clients_connect(&config_a, server_id.clone())
            .await
            .is_err(),
        "the module must report a 401 as a failed connection"
    );
    assert_eq!(
        connections::auth_hint_for_config(&config_a, &server_id).await,
        Some("credential_required")
    );
    assert_eq!(
        connections::last_error_for_config(&config_a, &server_id).await,
        None,
        "401 response text stays hidden; the typed auth hint carries the user-facing reason"
    );
    assert!(connections::auth_hint_for_config(&config_b, &server_id)
        .await
        .is_none());
    assert!(connections::last_error_for_config(&config_b, &server_id)
        .await
        .is_none());
}
