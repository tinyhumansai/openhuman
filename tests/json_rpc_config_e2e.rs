//! JSON-RPC E2E cases for the config namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;
use openhuman_core::platform::connectivity::rpc::{pick_listen_port_for_host_with, OccupiedByCore};

#[tokio::test]
async fn json_rpc_legacy_provider_type_preserves_default_endpoint() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    write_min_config(&home.join(".openhuman"), "http://127.0.0.1:9");
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    for (index, method) in [
        "openhuman.config_update_model_settings",
        "openhuman.inference_update_model_settings",
    ]
    .into_iter()
    .enumerate()
    {
        let updated = post_json_rpc(
            &rpc_base,
            41_250 + index as i64,
            method,
            json!({
                "cloud_providers": [{"slug": "legacy", "endpoint": "", "type": " anthropic ",
                    "default_model": "claude-sonnet-4"}]
            }),
        )
        .await;
        let snapshot = peel_logs_envelope(assert_no_jsonrpc_error(
            &updated,
            "legacy provider endpoint update",
        ));
        let path = snapshot["config_path"].as_str().expect("saved config path");
        let persisted: openhuman_core::config::Config =
            toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let provider = persisted
            .cloud_providers
            .iter()
            .find(|p| p.slug == "legacy")
            .unwrap();
        assert_eq!(
            provider.endpoint, "https://api.anthropic.com/v1",
            "{method}"
        );
        assert_eq!(
            provider.auth_style,
            openhuman_core::config::schema::AuthStyle::Anthropic
        );
        assert_eq!(provider.default_model.as_deref(), Some("claude-sonnet-4"));
        let updated = post_json_rpc(
            &rpc_base,
            41_260 + index as i64,
            method,
            json!({"cloud_providers": [{"slug": "legacy", "endpoint": "http://127.0.0.1:9/v1",
                "type": "anthropic", "auth_style": "bearer"}]}),
        )
        .await;
        let snapshot = peel_logs_envelope(assert_no_jsonrpc_error(
            &updated,
            "explicit provider values",
        ));
        let providers = snapshot["config"]["cloud_providers"].as_array().unwrap();
        let provider = providers.iter().find(|p| p["slug"] == "legacy").unwrap();
        assert_eq!(provider["endpoint"], "http://127.0.0.1:9/v1");
        assert_eq!(provider["auth_style"], "bearer");
    }
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_config_update_browser_settings_persists_backend() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_min_config(&openhuman_home, "http://127.0.0.1:9");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    let updated = post_json_rpc(
        &rpc_base,
        41_241,
        "openhuman.config_update_browser_settings",
        json!({
            "enabled": true,
            "backend": "playwright",
            "learn_from_tasks": false
        }),
    )
    .await;
    let updated_result =
        assert_no_jsonrpc_error(&updated, "config_update_browser_settings backend");
    let updated_snapshot = peel_logs_envelope(updated_result);
    assert_eq!(
        updated_snapshot
            .pointer("/config/browser/enabled")
            .and_then(Value::as_bool),
        Some(true),
        "browser enabled flag should persist in update response: {updated_snapshot}"
    );
    assert_eq!(
        updated_snapshot
            .pointer("/config/browser/backend")
            .and_then(Value::as_str),
        Some("playwright"),
        "browser backend should persist in update response: {updated_snapshot}"
    );

    let get = post_json_rpc(&rpc_base, 41_242, "openhuman.config_get", json!({})).await;
    let get_result = assert_no_jsonrpc_error(&get, "config_get");
    let snapshot = peel_logs_envelope(get_result);
    assert_eq!(
        snapshot
            .pointer("/config/browser/backend")
            .and_then(Value::as_str),
        Some("playwright"),
        "browser backend should persist in config_get response: {snapshot}"
    );
    assert_eq!(
        snapshot
            .pointer("/config/browser/learn_from_tasks")
            .and_then(Value::as_bool),
        Some(false),
        "switching learning off should persist in config_get response: {snapshot}"
    );

    let invalid = post_json_rpc(
        &rpc_base,
        41_243,
        "openhuman.config_update_browser_settings",
        json!({
            "backend": "netscape"
        }),
    )
    .await;
    assert_jsonrpc_error(&invalid, "invalid browser backend");

    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_reasoning_effort_persists_and_rejects_unknown_levels() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_min_config(&openhuman_home, "http://127.0.0.1:9");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // The composer's default: an alias is stored as the canonical wire token
    // and surfaces in the client config the UI loads.
    let updated = post_json_rpc(
        &rpc_base,
        41_251,
        "openhuman.config_update_runtime_settings",
        json!({ "reasoning_effort": "max" }),
    )
    .await;
    assert_no_jsonrpc_error(&updated, "config_update_runtime_settings reasoning_effort");
    let client = post_json_rpc(
        &rpc_base,
        41_252,
        "openhuman.inference_get_client_config",
        json!({}),
    )
    .await;
    let client_config = peel_logs_envelope(assert_no_jsonrpc_error(
        &client,
        "inference_get_client_config",
    ));
    assert_eq!(
        client_config
            .get("reasoning_effort")
            .and_then(Value::as_str),
        Some("xhigh"),
        "client config should report the saved thinking level: {client_config}"
    );

    let invalid_default = post_json_rpc(
        &rpc_base,
        41_253,
        "openhuman.config_update_runtime_settings",
        json!({ "reasoning_effort": "turbo" }),
    )
    .await;
    assert_jsonrpc_error(&invalid_default, "unknown default reasoning_effort");

    // A per-thread level that does not parse is rejected before any turn starts.
    let invalid_turn = post_json_rpc(
        &rpc_base,
        41_254,
        "openhuman.channel_web_chat",
        json!({
            "client_id": "reasoning-client",
            "thread_id": "reasoning-thread",
            "message": "hello",
            "reasoning_effort": "turbo",
        }),
    )
    .await;
    let err = assert_jsonrpc_error(&invalid_turn, "unknown turn reasoning_effort");
    assert!(
        err.to_string().contains("reasoning_effort"),
        "the rejection should name the bad param: {err}"
    );

    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_config_autonomy_settings_roundtrip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // GET → expect the default (20).
    let initial = post_json_rpc(
        &rpc_base,
        7001,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let initial_outer = assert_no_jsonrpc_error(&initial, "get_autonomy_settings initial");
    // assert_no_jsonrpc_error already strips the JSON-RPC envelope; one more hop
    // strips the into_cli_compatible_json wrapper to reach the payload fields.
    let initial_value = initial_outer
        .get("result")
        .and_then(|r| r.get("max_actions_per_hour"))
        .and_then(Value::as_u64);
    // Default is `u32::MAX` (functionally unlimited) — fresh installs should
    // not be rate-limited until the user opts into a ceiling. See the
    // autonomy schema for the rationale.
    assert_eq!(
        initial_value,
        Some(u32::MAX as u64),
        "expected default u32::MAX (unlimited), got envelope: {initial_outer}"
    );

    // UPDATE → 250.
    let update = post_json_rpc(
        &rpc_base,
        7002,
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 250 }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_autonomy_settings");

    // GET again → expect 250.
    let after = post_json_rpc(
        &rpc_base,
        7003,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let after_outer = assert_no_jsonrpc_error(&after, "get_autonomy_settings after");
    let after_value = after_outer
        .get("result")
        .and_then(|r| r.get("max_actions_per_hour"))
        .and_then(Value::as_u64);
    assert_eq!(
        after_value,
        Some(250),
        "expected 250 after update, got envelope: {after_outer}"
    );

    // Invalid value rejected — server returns JSON-RPC error envelope, not a result.
    // Upper bound was lifted to u32::MAX (the new "unlimited" sentinel that the
    // UI exposes as a preset), so the only rejected value is now zero.
    let bad = post_json_rpc(
        &rpc_base,
        7004,
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 0 }),
    )
    .await;
    let bad_err = assert_jsonrpc_error(&bad, "update_autonomy_settings bad value");
    let err_message = bad_err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("error object missing message: {bad_err}"));
    assert!(
        err_message.contains("unsigned integer >= 1"),
        "expected validation error in: {err_message}"
    );

    // auto_approve ("Always allow" allowlist) round-trips through the same
    // update/get path the Agent Access settings panel uses.
    let update_allow = post_json_rpc(
        &rpc_base,
        7005,
        "openhuman.config_update_autonomy_settings",
        json!({ "auto_approve": ["shell", "curl"] }),
    )
    .await;
    assert_no_jsonrpc_error(&update_allow, "update_autonomy_settings auto_approve");

    let after_allow = post_json_rpc(
        &rpc_base,
        7006,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let after_allow_outer =
        assert_no_jsonrpc_error(&after_allow, "get_autonomy_settings auto_approve");
    let allow_list: Vec<String> = after_allow_outer
        .get("result")
        .and_then(|r| r.get("auto_approve"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        allow_list,
        vec!["shell".to_string(), "curl".to_string()],
        "auto_approve allowlist should round-trip, got envelope: {after_allow_outer}"
    );

    // `auto_approve_all` (the blanket bypass) round-trips through the same
    // update/get path, and defaults to `false`.
    let initial_auto_all = initial_outer
        .get("result")
        .and_then(|r| r.get("auto_approve_all"))
        .and_then(Value::as_bool);
    assert_eq!(
        initial_auto_all,
        Some(false),
        "auto_approve_all should default to false, got envelope: {initial_outer}"
    );

    let update_auto_all = post_json_rpc(
        &rpc_base,
        7007,
        "openhuman.config_update_autonomy_settings",
        json!({ "auto_approve_all": true }),
    )
    .await;
    assert_no_jsonrpc_error(
        &update_auto_all,
        "update_autonomy_settings auto_approve_all",
    );

    let after_auto_all = post_json_rpc(
        &rpc_base,
        7008,
        "openhuman.config_get_autonomy_settings",
        json!({}),
    )
    .await;
    let after_auto_all_outer =
        assert_no_jsonrpc_error(&after_auto_all, "get_autonomy_settings auto_approve_all");
    let auto_all_value = after_auto_all_outer
        .get("result")
        .and_then(|r| r.get("auto_approve_all"))
        .and_then(Value::as_bool);
    assert_eq!(
        auto_all_value,
        Some(true),
        "auto_approve_all should round-trip to true, got envelope: {after_auto_all_outer}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// Issue #3100 — the agent/action timeout must be readable and writable over
/// RPC (the surface the Settings → Agent OS access "Action timeout" control
/// uses), with the same bounds the UI shows and a validation error on garbage.

#[tokio::test]
async fn json_rpc_config_agent_timeout_settings_roundtrip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    // No operator override so the config value drives the effective timeout.
    let _timeout_env_guard = EnvVarGuard::unset("OPENHUMAN_TOOL_TIMEOUT_SECS");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // GET → defaults: 120s, bounds 1..=3600, no env override.
    let initial = post_json_rpc(
        &rpc_base,
        7101,
        "openhuman.config_get_agent_settings",
        json!({}),
    )
    .await;
    let initial_outer = assert_no_jsonrpc_error(&initial, "get_agent_settings initial");
    let initial_result = initial_outer
        .get("result")
        .unwrap_or_else(|| panic!("missing result: {initial_outer}"));
    assert_eq!(
        initial_result
            .get("agent_timeout_secs")
            .and_then(Value::as_u64),
        Some(120),
        "default timeout should be 120, got: {initial_outer}"
    );
    assert_eq!(
        initial_result
            .get("min_timeout_secs")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        initial_result
            .get("max_timeout_secs")
            .and_then(Value::as_u64),
        Some(3600)
    );
    assert_eq!(
        initial_result.get("env_override").and_then(Value::as_bool),
        Some(false)
    );

    // UPDATE → 300s.
    let update = post_json_rpc(
        &rpc_base,
        7102,
        "openhuman.config_update_agent_settings",
        json!({ "agent_timeout_secs": 300 }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_agent_settings");

    // GET again → 300, and the runtime-effective value tracks it.
    let after = post_json_rpc(
        &rpc_base,
        7103,
        "openhuman.config_get_agent_settings",
        json!({}),
    )
    .await;
    let after_outer = assert_no_jsonrpc_error(&after, "get_agent_settings after");
    let after_result = after_outer
        .get("result")
        .unwrap_or_else(|| panic!("missing result: {after_outer}"));
    assert_eq!(
        after_result
            .get("agent_timeout_secs")
            .and_then(Value::as_u64),
        Some(300),
        "expected 300 after update, got: {after_outer}"
    );
    assert_eq!(
        after_result
            .get("effective_timeout_secs")
            .and_then(Value::as_u64),
        Some(300),
        "effective timeout should track the saved value, got: {after_outer}"
    );

    // Invalid value (0 disables the timeout) → JSON-RPC error envelope.
    let bad = post_json_rpc(
        &rpc_base,
        7104,
        "openhuman.config_update_agent_settings",
        json!({ "agent_timeout_secs": 0 }),
    )
    .await;
    let bad_err = assert_jsonrpc_error(&bad, "update_agent_settings bad value");
    let err_message = bad_err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("error object missing message: {bad_err}"));
    assert!(
        err_message.contains("between"),
        "expected range validation error in: {err_message}"
    );

    // Restore the process-global timeout so later tests in this binary don't
    // inherit the 300s value set above (the AtomicU64 is per-process, not per-test).
    openhuman_core::tools::timeout::set_tool_timeout_secs(
        tinyagents_harness_tools::exec::timeout::DEFAULT_TIMEOUT_SECS,
    );

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Port-conflict recovery E2E
// ---------------------------------------------------------------------------
//
// Verifies that when the preferred core port is already occupied, the RPC
// stack starts successfully on a fallback port and remains fully reachable.
// Use an ephemeral preferred port so this remains isolated from a developer's
// already-running core on the standard port.

#[tokio::test]
async fn port_conflict_recovery_core_starts_on_fallback_port_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;

    // ── 1. occupy an ephemeral preferred port with a dummy listener ───────
    let blocker = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind blocker on an ephemeral port for e2e test");
    blocker
        .set_nonblocking(true)
        .expect("set blocker non-blocking");
    let preferred = blocker.local_addr().expect("blocker local addr").port();
    let fallbacks: Vec<u16> = (1..=10)
        .filter_map(|delta| preferred.checked_add(delta))
        .collect();

    // ── 2. pick_listen_port should fall back to a neighbouring port ───────
    let pick_result =
        pick_listen_port_for_host_with("127.0.0.1", preferred, OccupiedByCore::Fallback)
            .await
            .expect("pick_listen_port must succeed when port is occupied by non-OpenHuman process");
    assert!(
        pick_result.fallback_from.is_some(),
        "expected fallback_from to be Some({preferred}) when preferred port is occupied, got None"
    );
    assert_eq!(
        pick_result.fallback_from,
        Some(preferred),
        "fallback_from should record the originally preferred port"
    );
    let fallback_port = pick_result.port;
    assert!(
        fallbacks.contains(&fallback_port),
        "fallback port {fallback_port} should be one of {fallbacks:?}"
    );

    // ── 3. serve the core router on the fallback listener ────────────────
    ensure_test_rpc_auth();
    let router = build_core_http_router(false);
    let listener =
        tokio::net::TcpListener::from_std(pick_result.listener.into_std().expect("into_std"))
            .expect("from_std");
    let addr: SocketAddr = listener.local_addr().expect("local_addr");
    let rpc_join = tokio::spawn(async move { axum::serve(listener, router).await });
    let rpc_base = format!("http://{addr}");
    assert_eq!(
        addr.port(),
        fallback_port,
        "server should be on the fallback port"
    );

    // ── 4. RPC health-check on the fallback port ─────────────────────────
    let diag = post_json_rpc(&rpc_base, 26170, "openhuman.connectivity_diag", json!({})).await;
    // JSON-RPC result envelope → inner cli-compatible wrapper → diag payload.
    // post_json_rpc returns the raw JSON-RPC response; assert_no_jsonrpc_error
    // unwraps the outer "result". The inner value is {"logs":[...],"result":{"diag":{...}}},
    // so we need one more "result" hop before accessing "diag".
    let outer = assert_no_jsonrpc_error(&diag, "connectivity_diag on fallback port");
    let inner = outer.get("result").unwrap_or_else(|| {
        panic!("connectivity_diag outer result missing 'result' key; got: {outer}")
    });
    let diag_payload = inner.get("diag").unwrap_or_else(|| {
        panic!("connectivity_diag inner result missing 'diag' key; got: {inner}")
    });
    assert!(
        diag_payload.get("sidecar_pid").is_some(),
        "connectivity_diag should return sidecar_pid field; got: {diag_payload}"
    );
    assert!(
        diag_payload.get("listen_port").is_some(),
        "connectivity_diag should return listen_port field; got: {diag_payload}"
    );

    rpc_join.abort();

    // ── 5. drop blocker — verify the preferred port is now free ──────────
    drop(blocker);

    let after_drop =
        pick_listen_port_for_host_with("127.0.0.1", preferred, OccupiedByCore::Fallback)
            .await
            .expect("pick_listen_port should succeed with 7788 free");
    assert_eq!(
        after_drop.port, preferred,
        "after releasing the blocker, pick_listen_port should bind directly on the preferred port"
    );
    assert!(
        after_drop.fallback_from.is_none(),
        "fallback_from should be None when 7788 is free"
    );
    // Release the listener so the port is not held across tests.
    drop(after_drop.listener);
}
