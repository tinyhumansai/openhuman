//! Multi-server MCP integration tests through OpenHuman's current RPC adapters.
//!
//! TinyMCP owns the install store and connections. These tests use its
//! checksum-pinned module artifact through the host's public config and
//! registry operations, with the Cargo-built MCP stub as the local server.

#![cfg(feature = "mcp")]

use openhuman_core::config::{
    schema::{ModuleOverride, ModulesConfig},
    Config,
};
use serde_json::{json, Map, Value};

const MODULE_ARTIFACT_ENV: &str = "OPENHUMAN_TINYMCP_TEST_MODULE";

fn fresh_workspace_config() -> (tempfile::TempDir, Config) {
    let workspace = tempfile::tempdir().expect("temporary MCP workspace");
    let artifact = std::env::var_os(MODULE_ARTIFACT_ENV)
        .map(std::path::PathBuf::from)
        .or_else(|| {
            openhuman_core::modules::local_artifact("tinymcp").map(std::path::PathBuf::from)
        })
        .expect("the MCP test runner supplies the verified TinyMCP artifact")
        .canonicalize()
        .expect("TinyMCP artifact exists");
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        modules: ModulesConfig {
            enabled: true,
            allow_download: false,
            overrides: vec![ModuleOverride::with_path(
                "tinymcp",
                artifact.to_string_lossy().into_owned(),
            )],
            ..ModulesConfig::default()
        },
        ..Config::default()
    };
    (workspace, config)
}

fn stdio_server(command: impl Into<String>, enabled: bool) -> Value {
    json!({
        "command": command.into(),
        "args": [],
        "enabled": enabled,
    })
}

fn stub_server(enabled: bool) -> Value {
    stdio_server(env!("CARGO_BIN_EXE_test-mcp-stub"), enabled)
}

async fn install_servers(
    config: &Config,
    servers: Map<String, Value>,
) -> std::collections::HashMap<String, String> {
    openhuman_core::mcp::registry::config_ops::mcp_clients_config_set(
        config,
        json!({ "mcpServers": servers }),
    )
    .await
    .expect("apply the server document through TinyMCP");

    let status = openhuman_core::mcp::registry::ops::mcp_clients_status(config)
        .await
        .expect("read installed servers from TinyMCP");
    status.value["servers"]
        .as_array()
        .expect("status contains a server array")
        .iter()
        .map(|server| {
            let name = server["qualified_name"]
                .as_str()
                .expect("status contains the qualified server name")
                .to_owned();
            let id = server["server_id"]
                .as_str()
                .expect("status contains the module-owned server id")
                .to_owned();
            (name, id)
        })
        .collect()
}

async fn connect(config: &Config, server_id: &str) -> Result<Value, String> {
    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(config, server_id.to_owned(), true)
        .await?;
    openhuman_core::mcp::registry::ops::mcp_clients_connect(config, server_id.to_owned())
        .await
        .map(|outcome| outcome.value)
}

async fn call_echo(config: &Config, server_id: &str, message: &str) -> String {
    let outcome = openhuman_core::mcp::registry::ops::mcp_clients_tool_call(
        config,
        server_id.to_owned(),
        "echo".to_owned(),
        json!({ "message": message }),
    )
    .await
    .expect("call the stub tool through TinyMCP");
    assert_eq!(outcome.value["is_error"], false, "{outcome:?}");
    extract_echo_text(&outcome.value["result"]).to_owned()
}

fn extract_echo_text(result: &Value) -> &str {
    result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

async fn disconnect(config: &Config, server_id: &str) {
    openhuman_core::mcp::registry::ops::mcp_clients_disconnect(config, server_id.to_owned())
        .await
        .expect("disconnect the module-owned MCP session");
}

#[tokio::test]
async fn two_servers_same_tool_name_no_collision() {
    let (_workspace, config) = fresh_workspace_config();
    let servers = Map::from_iter([
        ("@multi-test/echo-a".to_owned(), stub_server(false)),
        ("@multi-test/echo-b".to_owned(), stub_server(false)),
    ]);
    let installed = install_servers(&config, servers).await;
    let server_a = &installed["@multi-test/echo-a"];
    let server_b = &installed["@multi-test/echo-b"];
    assert_ne!(server_a, server_b, "each install has its own server id");

    for server_id in [server_a, server_b] {
        let connected = connect(&config, server_id)
            .await
            .expect("connect stub server");
        assert_eq!(connected["tools"].as_array().unwrap().len(), 1);
        assert_eq!(connected["tools"][0]["name"], "echo");
    }

    let all_tools =
        openhuman_core::mcp::registry::connections::all_connected_tools_for_config(&config).await;
    let tools_a: Vec<_> = all_tools
        .iter()
        .filter(|(id, _, _)| id == server_a)
        .collect();
    let tools_b: Vec<_> = all_tools
        .iter()
        .filter(|(id, _, _)| id == server_b)
        .collect();
    assert_eq!(tools_a.len(), 1, "server A contributes one tool");
    assert_eq!(tools_b.len(), 1, "server B contributes one tool");
    assert_eq!(tools_a[0].2.name, "echo");
    assert_eq!(tools_b[0].2.name, "echo");

    let statuses = openhuman_core::mcp::registry::ops::mcp_clients_status(&config)
        .await
        .expect("read server status");
    for server_id in [server_a, server_b] {
        let status = statuses.value["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|status| status["server_id"] == *server_id)
            .expect("connected server remains in status");
        assert_eq!(status["status"], "connected");
        assert_eq!(status["tool_count"], 1);
    }

    assert_eq!(
        call_echo(&config, server_a, "payload-for-a").await,
        "payload-for-a"
    );
    assert_eq!(
        call_echo(&config, server_b, "payload-for-b").await,
        "payload-for-b"
    );
    disconnect(&config, server_a).await;
    disconnect(&config, server_b).await;
}

#[tokio::test]
async fn tool_calls_route_to_the_correct_server() {
    let (_workspace, config) = fresh_workspace_config();
    let servers = Map::from_iter([
        ("@route-test/echo-a".to_owned(), stub_server(false)),
        ("@route-test/echo-b".to_owned(), stub_server(false)),
    ]);
    let installed = install_servers(&config, servers).await;
    let server_a = &installed["@route-test/echo-a"];
    let server_b = &installed["@route-test/echo-b"];
    connect(&config, server_a).await.expect("connect server A");
    connect(&config, server_b).await.expect("connect server B");

    let text_a = call_echo(&config, server_a, "payload-for-a").await;
    let text_b = call_echo(&config, server_b, "payload-for-b").await;
    assert_eq!(text_a, "payload-for-a");
    assert_eq!(text_b, "payload-for-b");
    assert_ne!(text_a, text_b, "the server id determines the destination");

    disconnect(&config, server_a).await;
    disconnect(&config, server_b).await;
}

#[tokio::test]
async fn failed_connect_does_not_block_healthy_peer() {
    let (_workspace, config) = fresh_workspace_config();
    let servers = Map::from_iter([
        (
            "@isolation-test/bad".to_owned(),
            stdio_server("/this/path/does/not/exist/bad-mcp", false),
        ),
        ("@isolation-test/good".to_owned(), stub_server(false)),
    ]);
    let installed = install_servers(&config, servers).await;
    let bad = &installed["@isolation-test/bad"];
    let good = &installed["@isolation-test/good"];

    assert!(
        connect(&config, bad).await.is_err(),
        "bad server refuses to connect"
    );
    let connected = connect(&config, good)
        .await
        .expect("good server connects independently");
    assert_eq!(connected["tools"].as_array().unwrap().len(), 1);

    let statuses = openhuman_core::mcp::registry::ops::mcp_clients_status(&config)
        .await
        .expect("read status after isolated failure");
    let status_for = |id: &str| {
        statuses.value["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|status| status["server_id"] == id)
            .expect("server remains visible after connect attempt")
    };
    assert_eq!(status_for(bad)["status"], "error");
    assert!(status_for(bad)["last_error"]
        .as_str()
        .is_some_and(|error| !error.is_empty()));
    assert_eq!(status_for(good)["status"], "connected");
    assert!(status_for(good)["last_error"].is_null());

    let all_tools =
        openhuman_core::mcp::registry::connections::all_connected_tools_for_config(&config).await;
    assert!(all_tools.iter().all(|(id, _, _)| id != bad));
    assert!(all_tools
        .iter()
        .any(|(id, _, tool)| id == good && tool.name == "echo"));
    assert_eq!(
        call_echo(&config, good, "isolation-check").await,
        "isolation-check"
    );
    disconnect(&config, good).await;
}

#[tokio::test]
async fn disabled_server_contributes_no_tools_to_agent_surface() {
    let (_workspace, config) = fresh_workspace_config();
    let servers = Map::from_iter([
        ("@disabled-test/live".to_owned(), stub_server(false)),
        ("@disabled-test/quiet".to_owned(), stub_server(false)),
    ]);
    let installed = install_servers(&config, servers).await;
    let live = &installed["@disabled-test/live"];
    let quiet = &installed["@disabled-test/quiet"];
    connect(&config, live).await.expect("live server connects");

    assert!(
        openhuman_core::mcp::registry::ops::mcp_clients_connect(&config, quiet.to_owned())
            .await
            .is_err(),
        "an explicitly disabled server must refuse connection"
    );
    let statuses = openhuman_core::mcp::registry::ops::mcp_clients_status(&config)
        .await
        .expect("read server status");
    let status_for = |id: &str| {
        statuses.value["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|status| status["server_id"] == id)
            .expect("server remains visible in status")
    };
    assert_eq!(status_for(live)["status"], "connected");
    assert_eq!(status_for(quiet)["status"], "disabled");

    let all_tools =
        openhuman_core::mcp::registry::connections::all_connected_tools_for_config(&config).await;
    assert!(all_tools
        .iter()
        .any(|(id, _, tool)| id == live && tool.name == "echo"));
    assert!(all_tools.iter().all(|(id, _, _)| id != quiet));
    assert_eq!(
        call_echo(&config, live, "live-while-quiet").await,
        "live-while-quiet"
    );
    disconnect(&config, live).await;
}
