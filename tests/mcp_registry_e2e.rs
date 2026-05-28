//! End-to-end test for the `mcp_registry` connection lifecycle.
//!
//! Hermetic: spawns the `test-mcp-stub` binary (built alongside this test
//! by Cargo and exposed via `CARGO_BIN_EXE_test-mcp-stub`) as the MCP
//! subprocess. No npx, no network. Validates that
//! `store::insert_server` → `connections::connect` → `connections::call_tool`
//! → `connections::disconnect` round-trips correctly through the unified
//! `mcp_client::McpStdioClient` transport.

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::mcp_registry::connections;
use openhuman_core::openhuman::mcp_registry::store;
use openhuman_core::openhuman::mcp_registry::types::{CommandKind, InstalledServer, Transport};

fn fresh_workspace_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn make_installed_server() -> InstalledServer {
    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");
    InstalledServer {
        server_id: format!("test-{}", uuid::Uuid::new_v4()),
        qualified_name: "@openhuman-test/echo".to_string(),
        display_name: "Test Echo".to_string(),
        description: Some("Stub MCP server used by mcp_registry_e2e tests.".into()),
        icon_url: None,
        command_kind: CommandKind::Binary,
        command: stub_path.to_string(),
        args: Vec::new(),
        env_keys: Vec::new(),
        config: None,
        installed_at: 0,
        last_connected_at: None,
        transport: Transport::Stdio,
    }
}

#[tokio::test]
async fn connect_lists_one_tool_then_disconnect() {
    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();

    // Insert into the store so `all_status` (which reads from store) sees it,
    // and so a follow-up `boot::spawn_installed_servers` would pick it up.
    store::insert_server(&cfg, &server).expect("insert installed server");

    // Connect: spawns the stub subprocess and runs `initialize` + `tools/list`.
    let tools = connections::connect(&cfg, &server)
        .await
        .expect("connect succeeds");
    assert_eq!(tools.len(), 1, "stub advertises one tool");
    assert_eq!(tools[0].name, "echo");
    assert!(tools[0].input_schema.is_object());

    // Status reflects the live connection.
    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .expect("status entry present");
    assert_eq!(mine.tool_count, 1);

    // Call the `echo` tool and verify the response payload.
    let result = connections::call_tool(
        &server.server_id,
        "echo",
        serde_json::json!({ "message": "hello mcp" }),
    )
    .await
    .expect("call_tool succeeds");

    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert_eq!(text, "hello mcp", "echo tool returns the input verbatim");

    // Disconnect: removes from the registry and closes the subprocess.
    let removed = connections::disconnect(&server.server_id).await;
    assert!(removed, "disconnect drops the live connection");

    // Subsequent call fails because the server_id is no longer connected.
    let err = connections::call_tool(
        &server.server_id,
        "echo",
        serde_json::json!({ "message": "post-disconnect" }),
    )
    .await
    .expect_err("call_tool fails after disconnect");
    assert!(err.contains("not connected"));
}

#[tokio::test]
async fn unknown_tool_call_returns_error() {
    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();

    store::insert_server(&cfg, &server).expect("insert installed server");

    connections::connect(&cfg, &server).await.expect("connect");

    let err = connections::call_tool(&server.server_id, "does_not_exist", serde_json::json!({}))
        .await
        .expect_err("stub rejects unknown tools");
    assert!(
        err.to_lowercase().contains("unknown tool") || err.contains("error"),
        "expected unknown-tool error, got: {err}"
    );

    let _ = connections::disconnect(&server.server_id).await;
}
