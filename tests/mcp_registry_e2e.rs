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
        enabled: true,
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

#[tokio::test]
async fn failed_connect_records_last_error() {
    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.command = "/this/path/does/not/exist".to_string();

    store::insert_server(&cfg, &server).expect("insert installed server");

    let err = connections::connect(&cfg, &server)
        .await
        .expect_err("connect should fail for bogus command");
    assert!(!err.to_string().is_empty());

    let recorded = connections::last_error_for(&server.server_id).await;
    assert!(
        recorded.is_some(),
        "LAST_ERRORS must hold the connect failure for server_id={}",
        server.server_id
    );
}

#[tokio::test]
async fn successful_connect_clears_last_error() {
    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.command = "/nonexistent".to_string();
    let _ = connections::connect(&cfg, &server).await;
    assert!(connections::last_error_for(&server.server_id)
        .await
        .is_some());

    server.command = env!("CARGO_BIN_EXE_test-mcp-stub").to_string();
    connections::connect(&cfg, &server)
        .await
        .expect("real connect succeeds");
    assert!(
        connections::last_error_for(&server.server_id)
            .await
            .is_none(),
        "successful connect must clear the prior error"
    );

    let _ = connections::disconnect(&server.server_id).await;
}

#[tokio::test]
async fn status_priority_disabled_outranks_connected() {
    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.enabled = false;
    store::insert_server(&cfg, &server).expect("insert");

    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .expect("status entry present");
    assert_eq!(
        mine.status.as_str(),
        "disabled",
        "disabled server reports `disabled` even before any connect attempt"
    );
    assert!(mine.last_error.is_none());
}

#[tokio::test]
async fn status_reflects_last_connect_error() {
    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.command = "/nonexistent".to_string();
    store::insert_server(&cfg, &server).expect("insert");

    let _ = connections::connect(&cfg, &server).await;
    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .unwrap();
    assert_eq!(mine.status.as_str(), "error");
    assert!(
        mine.last_error
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "last_error populated"
    );
}

#[tokio::test]
async fn boot_skips_disabled_servers_and_records_errors() {
    use openhuman_core::openhuman::mcp_registry::boot;

    let (_tmp, cfg) = fresh_workspace_config();

    // Server A: enabled, real stub → connects.
    let mut a = make_installed_server();
    a.server_id = format!("a-{}", uuid::Uuid::new_v4());
    store::insert_server(&cfg, &a).expect("insert a");

    // Server B: enabled but command does not exist → records error, doesn't crash boot.
    let mut b = make_installed_server();
    b.server_id = format!("b-{}", uuid::Uuid::new_v4());
    b.command = "/nonexistent-mcp".to_string();
    store::insert_server(&cfg, &b).expect("insert b");

    // Server C: disabled AND command is bogus. If boot ever attempts to
    // connect this server, the bogus command will fail and LAST_ERRORS will
    // hold an entry. The skip is the only way the post-boot last_error stays
    // None — so the assertion below proves the skip actually fired, not just
    // that the Disabled-priority logic masked the failure.
    let mut c = make_installed_server();
    c.server_id = format!("c-{}", uuid::Uuid::new_v4());
    c.enabled = false;
    c.command = "/nonexistent-disabled-server".to_string();
    store::insert_server(&cfg, &c).expect("insert c");

    boot::spawn_installed_servers(&cfg).await;

    // A is connected; B recorded an error; C never attempted (no error
    // recorded despite the bogus command).
    let statuses = connections::all_status(&cfg).await;
    let by_id = |id: &str| {
        statuses
            .iter()
            .find(|s| s.server_id == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id(&a.server_id).status.as_str(), "connected");
    assert_eq!(by_id(&b.server_id).status.as_str(), "error");
    assert_eq!(by_id(&c.server_id).status.as_str(), "disabled");
    assert!(
        connections::last_error_for(&c.server_id).await.is_none(),
        "disabled server with bogus command must not have been connect-attempted"
    );

    let _ = connections::disconnect(&a.server_id).await;
}

#[tokio::test]
async fn set_enabled_false_disconnects_running_server() {
    use openhuman_core::openhuman::mcp_registry::ops;

    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();
    store::insert_server(&cfg, &server).expect("insert");
    connections::connect(&cfg, &server).await.expect("connect");

    let outcome = ops::mcp_clients_set_enabled(&cfg, server.server_id.clone(), false)
        .await
        .expect("set_enabled ok");
    assert_eq!(outcome.value["enabled"], serde_json::json!(false));

    let loaded = store::get_server(&cfg, &server.server_id).unwrap();
    assert!(!loaded.enabled);
    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .unwrap();
    assert_eq!(mine.status.as_str(), "disabled");
}

#[tokio::test]
async fn connect_refuses_disabled_server() {
    use openhuman_core::openhuman::mcp_registry::ops;

    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.enabled = false;
    store::insert_server(&cfg, &server).expect("insert");

    let err = ops::mcp_clients_connect(&cfg, server.server_id.clone())
        .await
        .expect_err("connect must reject disabled server");
    assert!(err.to_lowercase().contains("disabled"), "got: {err}");
}

#[tokio::test]
async fn set_enabled_true_clears_disabled_status_but_does_not_auto_connect() {
    use openhuman_core::openhuman::mcp_registry::ops;

    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.enabled = false;
    store::insert_server(&cfg, &server).expect("insert");

    ops::mcp_clients_set_enabled(&cfg, server.server_id.clone(), true)
        .await
        .expect("set_enabled true ok");
    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .unwrap();
    assert_eq!(
        mine.status.as_str(),
        "disconnected",
        "re-enabling alone must not bring up the subprocess; the user calls connect explicitly"
    );
}

#[tokio::test]
async fn update_env_on_disabled_server_persists_but_does_not_reconnect() {
    use openhuman_core::openhuman::mcp_registry::ops;
    use std::collections::HashMap;

    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.enabled = false;
    store::insert_server(&cfg, &server).expect("insert");

    let mut env = HashMap::new();
    env.insert("API_KEY".to_string(), "deadbeef".to_string());

    let outcome = ops::mcp_clients_update_env(&cfg, server.server_id.clone(), env)
        .await
        .expect("update_env on disabled server returns Ok");
    assert_eq!(
        outcome.value["status"], "disabled",
        "disabled server reports status=disabled instead of reconnecting"
    );

    let statuses = connections::all_status(&cfg).await;
    let mine = statuses
        .iter()
        .find(|s| s.server_id == server.server_id)
        .unwrap();
    assert_eq!(mine.status.as_str(), "disabled");
}

// ── Reconnect supervisor (#3312) ───────────────────────────────────────────────

#[tokio::test]
async fn probe_alive_reflects_transport_liveness() {
    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();
    store::insert_server(&cfg, &server).expect("insert installed server");

    connections::connect(&cfg, &server).await.expect("connect");
    assert!(connections::is_connected(&server.server_id).await);
    assert!(
        connections::probe_alive(&server.server_id, std::time::Duration::from_secs(8)).await,
        "a live stub answers the tools/list probe"
    );

    connections::disconnect(&server.server_id).await;
    assert!(!connections::is_connected(&server.server_id).await);
    assert!(
        !connections::probe_alive(&server.server_id, std::time::Duration::from_secs(8)).await,
        "a disconnected server is not alive"
    );
}

#[tokio::test]
async fn supervisor_reconnects_a_dropped_server() {
    use openhuman_core::openhuman::mcp_registry::supervisor;

    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();
    store::insert_server(&cfg, &server).expect("insert installed server");

    // Bring it up, then simulate a silent transport drop by disconnecting while
    // it stays installed + enabled in the store.
    connections::connect(&cfg, &server).await.expect("connect");
    connections::disconnect(&server.server_id).await;
    assert!(!connections::is_connected(&server.server_id).await);

    // One supervisor tick should notice the enabled-but-disconnected server and
    // reconnect it.
    supervisor::run_single_tick_for_test(&cfg).await;

    assert!(
        connections::is_connected(&server.server_id).await,
        "supervisor reconnects a dropped-but-installed server"
    );
    assert!(connections::probe_alive(&server.server_id, std::time::Duration::from_secs(8)).await);

    connections::disconnect(&server.server_id).await;
}

#[tokio::test]
async fn supervisor_leaves_a_healthy_connection_intact() {
    use openhuman_core::openhuman::mcp_registry::supervisor;

    let (_tmp, cfg) = fresh_workspace_config();
    let server = make_installed_server();
    store::insert_server(&cfg, &server).expect("insert installed server");
    connections::connect(&cfg, &server).await.expect("connect");

    // A tick over a healthy server must keep it connected (probe succeeds → no
    // disconnect/reconnect churn).
    supervisor::run_single_tick_for_test(&cfg).await;
    assert!(connections::is_connected(&server.server_id).await);

    connections::disconnect(&server.server_id).await;
}

#[tokio::test]
async fn supervisor_skips_a_disabled_server() {
    use openhuman_core::openhuman::mcp_registry::supervisor;

    let (_tmp, cfg) = fresh_workspace_config();
    let mut server = make_installed_server();
    server.enabled = false;
    store::insert_server(&cfg, &server).expect("insert installed server");

    // A disabled server must never be connected by the supervisor.
    supervisor::run_single_tick_for_test(&cfg).await;
    assert!(
        !connections::is_connected(&server.server_id).await,
        "supervisor does not connect disabled servers"
    );
}
