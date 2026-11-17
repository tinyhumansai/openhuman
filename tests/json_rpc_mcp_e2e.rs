//! JSON-RPC E2E cases for the mcp namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

async fn install_mcp_test_storage(home: &Path) {
    let storage_url = format!("sqlite:{}", home.join("mcp-test-storage").display());
    let backend = openhuman_core::storage::open(&storage_url)
        .await
        .expect("open isolated MCP test storage backend");
    openhuman_core::storage::install(backend);
}

#[tokio::test]
#[cfg_attr(
    target_os = "linux",
    ignore = "registry stdio MCP servers need network; TinyBox refuses a network-sharing jail for untrusted code until it has isolated networking (https://github.com/tinyhumansai/tinybox/issues/109)"
)]
async fn mcp_clients_lifecycle() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("local");
    write_min_config(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. installed_list should start empty ─────────────────────────────────
    let list1 = post_json_rpc(
        &rpc_base,
        9901,
        "openhuman.mcp_clients_installed_list",
        json!({}),
    )
    .await;
    let list1_result = assert_no_jsonrpc_error(&list1, "mcp_clients_installed_list (initial)");
    let list1_body = peel_logs_envelope(list1_result);
    let installed = list1_body
        .get("installed")
        .and_then(Value::as_array)
        .expect("installed_list must return an 'installed' array");
    assert!(
        installed.is_empty(),
        "installed list should start empty: {installed:?}"
    );

    // ── 2. status should return empty servers ─────────────────────────────────
    let status1 = post_json_rpc(&rpc_base, 9902, "openhuman.mcp_clients_status", json!({})).await;
    let status1_result = assert_no_jsonrpc_error(&status1, "mcp_clients_status (initial)");
    let status1_body = peel_logs_envelope(status1_result);
    let servers = status1_body
        .get("servers")
        .and_then(Value::as_array)
        .expect("status must return 'servers' array");
    assert!(servers.is_empty(), "status should start empty: {servers:?}");

    // ── 3. uninstall a non-existent server is a no-op ────────────────────────
    let uninstall_missing = post_json_rpc(
        &rpc_base,
        9903,
        "openhuman.mcp_clients_uninstall",
        json!({ "server_id": "00000000-0000-0000-0000-000000000000" }),
    )
    .await;
    // Non-existent id: may return error or removed=false — both are acceptable.
    // We just verify it does not panic the server.
    assert!(
        uninstall_missing.get("result").is_some() || uninstall_missing.get("error").is_some(),
        "uninstall missing server should return result or error: {uninstall_missing}"
    );

    // ── 4. registry_search (schema validation — may not have network in CI) ───
    let search = post_json_rpc(
        &rpc_base,
        9904,
        "openhuman.mcp_clients_registry_search",
        json!({ "query": "test", "page": 1, "page_size": 5 }),
    )
    .await;
    // Result or error are both acceptable; method must be registered.
    assert!(
        search.get("result").is_some() || search.get("error").is_some(),
        "registry_search should return result or error: {search}"
    );

    // ── 5. connect on a non-installed server returns an error ─────────────────
    let connect_missing = post_json_rpc(
        &rpc_base,
        9905,
        "openhuman.mcp_clients_connect",
        json!({ "server_id": "00000000-0000-0000-0000-000000000001" }),
    )
    .await;
    assert!(
        connect_missing.get("error").is_some(),
        "connect on missing server should return error: {connect_missing}"
    );

    // ── 6. tool_call on a non-connected server returns is_error=true ─────────
    let tool_call_disconnected = post_json_rpc(
        &rpc_base,
        9906,
        "openhuman.mcp_clients_tool_call",
        json!({
            "server_id": "00000000-0000-0000-0000-000000000002",
            "tool_name": "search",
            "arguments": {}
        }),
    )
    .await;
    let tc_result =
        assert_no_jsonrpc_error(&tool_call_disconnected, "tool_call on disconnected server");
    let tc_body = peel_logs_envelope(tc_result);
    assert_eq!(
        tool_call_is_error(tc_body),
        Some(true),
        "tool_call on disconnected server should set is_error=true: {tc_body}"
    );

    // ── 7. disconnect on a non-connected server is a no-op ────────────────────
    let disconnect_noop = post_json_rpc(
        &rpc_base,
        9907,
        "openhuman.mcp_clients_disconnect",
        json!({ "server_id": "00000000-0000-0000-0000-000000000003" }),
    )
    .await;
    let disc_result = assert_no_jsonrpc_error(&disconnect_noop, "disconnect noop");
    let disc_body = peel_logs_envelope(disc_result);
    assert_eq!(
        disc_body.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "disconnect noop should return status=disconnected: {disc_body}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// MCP clients **happy path** over real JSON-RPC: declare in `mcp.json` →
/// connect → tool_call → update_env (reconnect) → disconnect against a real
/// stdio MCP subprocess (the `test-mcp-stub` binary).
///
/// No npx, no network, no catalog: the server is declared through
/// `mcp_clients_config_set` with a `command` pointing at the stub binary, which
/// is how a user adds any server now — the registry is browse-only.

#[tokio::test]
#[cfg_attr(
    target_os = "linux",
    ignore = "registry stdio MCP servers need network; TinyBox refuses a network-sharing jail for untrusted code until it has isolated networking (https://github.com/tinyhumansai/tinybox/issues/109)"
)]
async fn mcp_clients_declare_connect_tool_call_happy_path() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("local");
    write_min_config(&user_scoped_dir, &mock_origin);

    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. declaring the stub in mcp.json installs it ────────────────────────
    let declared = post_json_rpc(
        &rpc_base,
        9920,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo": { "command": stub_path } } }),
    )
    .await;
    let declared_body = peel_logs_envelope(assert_no_jsonrpc_error(
        &declared,
        "mcp_clients_config_set (declare)",
    ));
    assert_eq!(
        declared_body.get("added"),
        Some(&json!(["echo"])),
        "config_set should report the new server as added: {declared_body}"
    );
    assert_eq!(
        declared_body["mcpServers"]["echo"]["command"],
        json!(stub_path),
        "config_set should render the declared command back: {declared_body}"
    );
    let server_id = installed_server_id(&rpc_base, 9926, "echo").await;

    // ── 2. connect spawns the stub and lists its one `echo` tool ─────────────
    let connect = post_json_rpc(
        &rpc_base,
        9921,
        "openhuman.mcp_clients_connect",
        json!({ "server_id": server_id }),
    )
    .await;
    let connect_result = assert_no_jsonrpc_error(&connect, "mcp_clients_connect (happy path)");
    let connect_body = peel_logs_envelope(connect_result);
    assert_eq!(
        connect_body.get("status").and_then(Value::as_str),
        Some("connected"),
        "connect should report connected: {connect_body}"
    );
    let tools = connect_body
        .get("tools")
        .and_then(Value::as_array)
        .expect("connect returns a tools array");
    assert!(
        tools
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some("echo")),
        "stub should advertise the echo tool: {tools:?}"
    );

    // ── 3. tool_call echoes the input back, is_error=false ───────────────────
    let tool_call = post_json_rpc(
        &rpc_base,
        9922,
        "openhuman.mcp_clients_tool_call",
        json!({
            "server_id": server_id,
            "tool_name": "echo",
            "arguments": { "message": "hello over rpc" }
        }),
    )
    .await;
    let tc_result = assert_no_jsonrpc_error(&tool_call, "mcp_clients_tool_call (happy path)");
    let tc_body = peel_logs_envelope(tc_result);
    assert_eq!(
        tool_call_is_error(tc_body),
        Some(false),
        "echo tool_call should not be an error: {tc_body}"
    );
    let tc_inner = tc_body.get("result").unwrap_or(tc_body);
    assert!(
        tc_inner.to_string().contains("hello over rpc"),
        "echo tool_call should round-trip the input payload: {tc_inner}"
    );

    // ── 4. update_env reconfigures + reconnects (no uninstall/reinstall) ─────
    let update_env = post_json_rpc(
        &rpc_base,
        9923,
        "openhuman.mcp_clients_update_env",
        json!({ "server_id": server_id, "env": { "EXAMPLE_TOKEN": "rotated" } }),
    )
    .await;
    let ue_result = assert_no_jsonrpc_error(&update_env, "mcp_clients_update_env (happy path)");
    let ue_body = peel_logs_envelope(ue_result);
    assert_eq!(
        ue_body.get("status").and_then(Value::as_str),
        Some("connected"),
        "update_env should reconnect: {ue_body}"
    );
    let env_keys = ue_body
        .get("env_keys")
        .and_then(Value::as_array)
        .expect("update_env returns env_keys");
    assert!(
        env_keys.iter().any(|k| k.as_str() == Some("EXAMPLE_TOKEN")),
        "update_env should persist the new env key: {env_keys:?}"
    );

    // Verify the reconnected session is still functional: call echo again.
    let tool_call2 = post_json_rpc(
        &rpc_base,
        9925,
        "openhuman.mcp_clients_tool_call",
        json!({
            "server_id": server_id,
            "tool_name": "echo",
            "arguments": { "message": "hello after reconfigure" }
        }),
    )
    .await;
    let tc2_result =
        assert_no_jsonrpc_error(&tool_call2, "mcp_clients_tool_call (after update_env)");
    let tc2_body = peel_logs_envelope(tc2_result);
    assert_eq!(
        tool_call_is_error(tc2_body),
        Some(false),
        "echo tool_call after reconfigure should not be an error: {tc2_body}"
    );
    let tc2_inner = tc2_body.get("result").unwrap_or(tc2_body);
    assert!(
        tc2_inner.to_string().contains("hello after reconfigure"),
        "echo tool_call after reconfigure should round-trip the input payload: {tc2_inner}"
    );

    // ── 5. disconnect cleans up the subprocess ───────────────────────────────
    let disconnect = post_json_rpc(
        &rpc_base,
        9924,
        "openhuman.mcp_clients_disconnect",
        json!({ "server_id": server_id }),
    )
    .await;
    let disc_result = assert_no_jsonrpc_error(&disconnect, "mcp_clients_disconnect (happy path)");
    let disc_body = peel_logs_envelope(disc_result);
    assert_eq!(
        disc_body.get("status").and_then(Value::as_str),
        Some("disconnected"),
        "disconnect should report disconnected: {disc_body}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// MCP clients **reconfigure and retire** over real JSON-RPC (#7571): connect
/// and call through a configured client, change the module's client
/// configuration (registry credentials are part of it, so the host reopens the
/// module's configured object with a new configuration), prove the install
/// survives and answers again, then retire it by dropping it from `mcp.json`
/// and prove neither the install nor a call to it is left behind.

#[tokio::test]
#[cfg_attr(
    target_os = "linux",
    ignore = "registry stdio MCP servers need network; TinyBox refuses a network-sharing jail for untrusted code until it has isolated networking (https://github.com/tinyhumansai/tinybox/issues/109)"
)]
async fn mcp_clients_reconfigure_then_retire_over_rpc() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    write_min_config(&openhuman_home.join("users").join("local"), &mock_origin);

    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // connect -> list_tools -> call through the configured client
    let declared = post_json_rpc(
        &rpc_base,
        9950,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo-reconf": { "command": stub_path } } }),
    )
    .await;
    assert_no_jsonrpc_error(&declared, "mcp_clients_config_set (declare)");
    let server_id = installed_server_id(&rpc_base, 9951, "echo-reconf").await;
    let connect = post_json_rpc(
        &rpc_base,
        9952,
        "openhuman.mcp_clients_connect",
        json!({ "server_id": server_id }),
    )
    .await;
    assert_no_jsonrpc_error(&connect, "mcp_clients_connect");
    let listed = post_json_rpc(
        &rpc_base,
        9953,
        "openhuman.mcp_clients_list_tools",
        json!({ "server_id": server_id }),
    )
    .await;
    let listed = peel_logs_envelope(assert_no_jsonrpc_error(&listed, "list_tools"));
    assert!(
        listed.to_string().contains("echo"),
        "stub should list the echo tool: {listed}"
    );

    // reconfigure: a different client configuration reopens the configured object
    let reconf = post_json_rpc(
        &rpc_base,
        9954,
        "openhuman.mcp_clients_registry_settings_set",
        json!({ "mcp_official_base": "https://registry.reconfigure.example.test" }),
    )
    .await;
    assert_no_jsonrpc_error(&reconf, "registry_settings_set (reconfigure)");
    let settings = post_json_rpc(
        &rpc_base,
        9955,
        "openhuman.mcp_clients_registry_settings_get",
        json!({}),
    )
    .await;
    let settings = peel_logs_envelope(assert_no_jsonrpc_error(&settings, "settings_get"));
    assert_eq!(
        settings.get("mcp_official_base").and_then(Value::as_str),
        Some("https://registry.reconfigure.example.test"),
        "the reopened module serves the new configuration: {settings}"
    );
    // The install survives the reopen and is usable again after a connect.
    assert_eq!(
        installed_server_id(&rpc_base, 9956, "echo-reconf").await,
        server_id
    );
    let reconnect = post_json_rpc(
        &rpc_base,
        9957,
        "openhuman.mcp_clients_connect",
        json!({ "server_id": server_id }),
    )
    .await;
    assert_no_jsonrpc_error(&reconnect, "mcp_clients_connect (after reconfigure)");
    let echoed = post_json_rpc(
        &rpc_base,
        9958,
        "openhuman.mcp_clients_tool_call",
        json!({
            "server_id": server_id,
            "tool_name": "echo",
            "arguments": { "message": "after reconfigure" }
        }),
    )
    .await;
    let echoed = peel_logs_envelope(assert_no_jsonrpc_error(&echoed, "tool_call (reconfigured)"));
    assert_eq!(tool_call_is_error(echoed), Some(false), "{echoed}");
    assert!(echoed.to_string().contains("after reconfigure"), "{echoed}");

    // retire: dropping the entry uninstalls it and nothing answers for it
    let retired = post_json_rpc(
        &rpc_base,
        9959,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": {} }),
    )
    .await;
    let retired = peel_logs_envelope(assert_no_jsonrpc_error(&retired, "config_set (retire)"));
    assert_eq!(
        retired.get("removed"),
        Some(&json!(["echo-reconf"])),
        "{retired}"
    );
    let status = post_json_rpc(&rpc_base, 9960, "openhuman.mcp_clients_status", json!({})).await;
    let status = peel_logs_envelope(assert_no_jsonrpc_error(&status, "status (retired)"));
    assert_eq!(status["servers"], json!([]), "{status}");
    let gone = post_json_rpc(
        &rpc_base,
        9961,
        "openhuman.mcp_clients_tool_call",
        json!({ "server_id": server_id, "tool_name": "echo", "arguments": {} }),
    )
    .await;
    let gone_text = gone.to_string();
    assert!(
        gone.get("error").is_some() || gone_text.contains("\"is_error\":true"),
        "a retired install must not answer a call: {gone}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// `mcp_clients_set_enabled` smoke: declares a server, disables it via RPC,
/// and asserts the response carries `enabled=false` (issue #3196).

#[tokio::test]
async fn mcp_clients_set_enabled_smoke() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("local");
    write_min_config(&user_scoped_dir, &mock_origin);

    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. declare ───────────────────────────────────────────────────────────
    let declared = post_json_rpc(
        &rpc_base,
        9940,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo-set-enabled": { "command": stub_path } } }),
    )
    .await;
    assert_no_jsonrpc_error(&declared, "mcp_clients_config_set (set_enabled smoke)");
    let server_id = installed_server_id(&rpc_base, 9943, "echo-set-enabled").await;

    // ── 2. set_enabled=false ─────────────────────────────────────────────────
    let set_enabled = post_json_rpc(
        &rpc_base,
        9941,
        "openhuman.mcp_clients_set_enabled",
        json!({ "server_id": server_id, "enabled": false }),
    )
    .await;
    let se_result = assert_no_jsonrpc_error(&set_enabled, "mcp_clients_set_enabled (false)");
    let se_body = peel_logs_envelope(se_result);
    assert_eq!(
        se_body.get("enabled"),
        Some(&json!(false)),
        "set_enabled should report enabled=false: {se_body}"
    );
    assert_eq!(
        se_body.get("server_id").and_then(Value::as_str),
        Some(server_id.as_str()),
        "set_enabled should echo server_id: {se_body}"
    );

    // ── 3. set_enabled=true restores it ─────────────────────────────────────
    let set_enabled_true = post_json_rpc(
        &rpc_base,
        9942,
        "openhuman.mcp_clients_set_enabled",
        json!({ "server_id": server_id, "enabled": true }),
    )
    .await;
    let se_true_result =
        assert_no_jsonrpc_error(&set_enabled_true, "mcp_clients_set_enabled (true)");
    let se_true_body = peel_logs_envelope(se_true_result);
    assert_eq!(
        se_true_body.get("enabled"),
        Some(&json!(true)),
        "set_enabled=true should report enabled=true: {se_true_body}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// The `mcp.json` round trip: a document is a *replace* of the install store,
/// except for credentials, which are write-only.
///
/// (a) a second save that names one credential merges it over the stored set
/// and keeps the row (same `server_id`); (b) saving exactly what `config_get`
/// returned — which carries no values — changes nothing; (c) a key set to `""`
/// removes that one value; (d) dropping the entry uninstalls the server.

#[tokio::test]
async fn mcp_clients_config_round_trip_merges_credentials_and_removes_absent_servers() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("local");
    write_min_config(&user_scoped_dir, &mock_origin);

    let stub_path = env!("CARGO_BIN_EXE_test-mcp-stub");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. declare with two credentials; disabled so nothing is dialled ──────
    let first = post_json_rpc(
        &rpc_base,
        9960,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo-rt": {
            "command": stub_path,
            "env": { "TOKEN": "old", "KEEP": "v1" },
            "enabled": false
        } } }),
    )
    .await;
    let b1 = peel_logs_envelope(assert_no_jsonrpc_error(&first, "config_set (first)"));
    assert_eq!(b1.get("added"), Some(&json!(["echo-rt"])), "{b1}");
    // The read carries names and a flag, never a value.
    assert_eq!(b1["mcpServers"]["echo-rt"]["authConfigured"], json!(true));
    assert_eq!(
        b1["mcpServers"]["echo-rt"]["envKeys"],
        json!(["KEEP", "TOKEN"])
    );
    assert_eq!(b1["mcpServers"]["echo-rt"]["enabled"], json!(false));
    assert!(
        b1["mcpServers"]["echo-rt"].get("env").is_none(),
        "a read must not echo credential values: {b1}"
    );
    let server_id = installed_server_id(&rpc_base, 9961, "echo-rt").await;

    // ── 2. a save naming one key merges it and keeps the row ─────────────────
    let second = post_json_rpc(
        &rpc_base,
        9962,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo-rt": {
            "command": stub_path,
            "env": { "TOKEN": "new" },
            "enabled": false
        } } }),
    )
    .await;
    let b2 = peel_logs_envelope(assert_no_jsonrpc_error(&second, "config_set (rotate)"));
    assert_eq!(b2.get("updated"), Some(&json!(["echo-rt"])), "{b2}");
    assert_eq!(b2.get("added"), Some(&json!([])), "{b2}");
    assert_eq!(
        b2["mcpServers"]["echo-rt"]["envKeys"],
        json!(["KEEP", "TOKEN"]),
        "rotating one key must keep the other: {b2}"
    );
    assert_eq!(
        installed_server_id(&rpc_base, 9963, "echo-rt").await,
        server_id,
        "a credential change must not replace the row"
    );

    // ── 3. saving the read back verbatim changes nothing ─────────────────────
    let read = post_json_rpc(
        &rpc_base,
        9964,
        "openhuman.mcp_clients_config_get",
        json!({}),
    )
    .await;
    let read_body = peel_logs_envelope(assert_no_jsonrpc_error(&read, "config_get"));
    let resave = post_json_rpc(
        &rpc_base,
        9965,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": read_body["mcpServers"].clone() }),
    )
    .await;
    let b3 = peel_logs_envelope(assert_no_jsonrpc_error(&resave, "config_set (re-save)"));
    assert_eq!(
        b3.get("updated"),
        Some(&json!([])),
        "a re-save is not an edit: {b3}"
    );
    assert_eq!(
        b3["mcpServers"]["echo-rt"]["envKeys"],
        json!(["KEEP", "TOKEN"])
    );

    // ── 4. an empty value removes that one credential ────────────────────────
    let clear = post_json_rpc(
        &rpc_base,
        9966,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "echo-rt": {
            "command": stub_path,
            "env": { "KEEP": "" },
            "enabled": false
        } } }),
    )
    .await;
    let b4 = peel_logs_envelope(assert_no_jsonrpc_error(&clear, "config_set (clear one)"));
    assert_eq!(
        b4["mcpServers"]["echo-rt"]["envKeys"],
        json!(["TOKEN"]),
        "{b4}"
    );

    // ── 5. a document without the entry uninstalls it ────────────────────────
    let drop = post_json_rpc(
        &rpc_base,
        9967,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": {} }),
    )
    .await;
    let b5 = peel_logs_envelope(assert_no_jsonrpc_error(&drop, "config_set (remove)"));
    assert_eq!(b5.get("removed"), Some(&json!(["echo-rt"])), "{b5}");
    assert_eq!(b5["mcpServers"], json!({}), "{b5}");

    // ── 6. a refused document is redacted and leaves the store unchanged ────
    let refused = post_json_rpc(
        &rpc_base,
        9968,
        "openhuman.mcp_clients_config_set",
        json!({ "mcpServers": { "bad": { "url": "https://x.test", "command": "npx" } } }),
    )
    .await;
    let message = refused["error"]["message"]
        .as_str()
        .expect("a refusal is a JSON-RPC error");
    assert_eq!(
        message, "MODULE_CALL_REPORTED: module execution failed",
        "the host reports sanitized module errors instead of exposing remote text"
    );
    let after_refusal = post_json_rpc(
        &rpc_base,
        9969,
        "openhuman.mcp_clients_config_get",
        json!({}),
    )
    .await;
    let after_refusal = peel_logs_envelope(assert_no_jsonrpc_error(
        &after_refusal,
        "config_get (after refusal)",
    ));
    assert_eq!(
        after_refusal["mcpServers"],
        json!({}),
        "a refused replacement leaves the install store unchanged"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// The `server_id` the store gave a server declared under `name`.
async fn installed_server_id(rpc_base: &str, id: i64, name: &str) -> String {
    let listed = post_json_rpc(
        rpc_base,
        id,
        "openhuman.mcp_clients_installed_list",
        json!({}),
    )
    .await;
    let body = peel_logs_envelope(assert_no_jsonrpc_error(
        &listed,
        "mcp_clients_installed_list",
    ));
    body.get("installed")
        .and_then(Value::as_array)
        .expect("installed list")
        .iter()
        .find(|s| s.get("qualified_name").and_then(Value::as_str) == Some(name))
        .and_then(|s| s.get("server_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("`{name}` should be installed: {body}"))
        .to_string()
}

/// Registry settings RPC: the getter reports `*_set` booleans without ever
/// echoing secret values; the setter persists and clears them (issue #3039
/// gap A6).

#[tokio::test]
async fn mcp_clients_registry_settings_roundtrip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    install_mcp_test_storage(home).await;
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _smithery_guard = EnvVarGuard::unset("SMITHERY_API_KEY");
    let _official_token_guard = EnvVarGuard::unset("MCP_OFFICIAL_REGISTRY_TOKEN");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    write_min_config(&openhuman_home.join("users").join("local"), &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Initially nothing is set.
    let get1 = post_json_rpc(
        &rpc_base,
        9930,
        "openhuman.mcp_clients_registry_settings_get",
        json!({}),
    )
    .await;
    let get1_result = assert_no_jsonrpc_error(&get1, "registry_settings_get (initial)");
    let get1_body = peel_logs_envelope(get1_result);
    assert_eq!(get1_body.get("smithery_api_key_set"), Some(&json!(false)));
    assert_eq!(get1_body.get("mcp_official_token_set"), Some(&json!(false)));

    // Set a key + base override.
    let set1 = post_json_rpc(
        &rpc_base,
        9931,
        "openhuman.mcp_clients_registry_settings_set",
        json!({
            "smithery_api_key": "sk-secret-value",
            "mcp_official_base": "https://registry.example.test"
        }),
    )
    .await;
    let set1_result = assert_no_jsonrpc_error(&set1, "registry_settings_set (set)");
    let set1_body = peel_logs_envelope(set1_result);
    assert_eq!(set1_body.get("smithery_api_key_set"), Some(&json!(true)));
    assert_eq!(
        set1_body.get("mcp_official_base").and_then(Value::as_str),
        Some("https://registry.example.test"),
    );
    // The secret value is NEVER echoed back anywhere in the response.
    assert!(
        !set1.to_string().contains("sk-secret-value"),
        "registry_settings_set must not echo the secret value"
    );

    // Read-after-write: verify getter reflects the persisted state.
    let get2 = post_json_rpc(
        &rpc_base,
        9933,
        "openhuman.mcp_clients_registry_settings_get",
        json!({}),
    )
    .await;
    let get2_result = assert_no_jsonrpc_error(&get2, "registry_settings_get (after set)");
    let get2_body = peel_logs_envelope(get2_result);
    assert_eq!(get2_body.get("smithery_api_key_set"), Some(&json!(true)));
    assert_eq!(
        get2_body.get("mcp_official_base").and_then(Value::as_str),
        Some("https://registry.example.test"),
    );
    // Getter must never return the raw secret.
    assert!(
        !get2.to_string().contains("sk-secret-value"),
        "registry_settings_get must not return the secret value"
    );

    // Clearing with an empty string flips the boolean back to false.
    let set2 = post_json_rpc(
        &rpc_base,
        9932,
        "openhuman.mcp_clients_registry_settings_set",
        json!({ "smithery_api_key": "" }),
    )
    .await;
    let set2_result = assert_no_jsonrpc_error(&set2, "registry_settings_set (clear)");
    let set2_body = peel_logs_envelope(set2_result);
    assert_eq!(set2_body.get("smithery_api_key_set"), Some(&json!(false)));
    // The base override persists across the clear of an unrelated field.
    assert_eq!(
        set2_body.get("mcp_official_base").and_then(Value::as_str),
        Some("https://registry.example.test"),
    );

    // Read-after-clear: verify getter reflects the cleared state.
    let get3 = post_json_rpc(
        &rpc_base,
        9934,
        "openhuman.mcp_clients_registry_settings_get",
        json!({}),
    )
    .await;
    let get3_result = assert_no_jsonrpc_error(&get3, "registry_settings_get (after clear)");
    let get3_body = peel_logs_envelope(get3_result);
    assert_eq!(get3_body.get("smithery_api_key_set"), Some(&json!(false)));
    // Base override should persist even after clearing the API key.
    assert_eq!(
        get3_body.get("mcp_official_base").and_then(Value::as_str),
        Some("https://registry.example.test"),
    );

    mock_join.abort();
    rpc_join.abort();
}
