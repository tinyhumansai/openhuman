use super::*;

#[test]
fn empty_uncurated_toolkits_message_uses_contract_catalog() {
    // Used to register a stub `ComposioProvider` under the engine's
    // provider registry and assert a toolkit answered only by that
    // provider's `curated_tools()` (not the static catalog) still counted
    // as catalogued. tinymemory v1.13.4 deleted `ComposioProvider` and the
    // registry outright with no replacement (see `providers`'s module
    // docs) — `uncatalogued_toolkits` now consults `catalog_for_toolkit`
    // alone, so this asserts the same "catalogued toolkit produces no
    // message" behaviour against a real catalogued toolkit instead of an
    // injected one.
    assert!(empty_uncurated_toolkits_message(&["gmail".to_string()]).is_none());
    assert!(empty_uncurated_toolkits_message(&["not-a-real-toolkit-xyz".to_string()]).is_some());
}

#[test]
fn render_tools_markdown_handles_empty_response() {
    use crate::openhuman::integrations::composio::types::ComposioToolsResponse;

    let resp = ComposioToolsResponse { tools: vec![] };
    let md = render_tools_markdown(&resp);
    assert!(md.contains("No composio tools available"));
}

#[test]
fn execute_tool_resolves_to_direct_kind_when_mode_is_direct() {
    // The whole point of fix #1710: the live `config.composio.mode`
    // governs which client variant `ComposioExecuteTool` dispatches
    // through. The pre-baked-client version of this code would have
    // routed through the backend regardless — silent direct-mode
    // breakage. We assert by independently calling the same factory the
    // tool calls per-execute.
    let config = direct_mode_config();
    let kind = crate::openhuman::integrations::composio::client::create_composio_client(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT,
        "factory should pick the direct variant when mode=direct"
    );
}

#[test]
fn execute_tool_resolves_to_backend_kind_when_mode_is_backend() {
    // Reverse of the above — confirms the backend path still wins when
    // the user is on default (mode = "backend") and a session token is
    // present. Without the token, `create_composio_client` returns
    // Err("no backend session"); store one to get past that gate.
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::openhuman::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    let kind = crate::openhuman::integrations::composio::client::create_composio_client(&config)
        .expect("backend mode with session token must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_BACKEND,
        "factory should pick the backend variant when mode=backend"
    );
}

#[tokio::test]
async fn list_tools_in_direct_mode_returns_empty_without_hitting_backend() {
    // In direct mode `composio_list_tools` deliberately returns an empty
    // `ComposioToolsResponse` and logs an info-level note (matches the
    // ops.rs pattern for list_toolkits/list_connections). The critical
    // assertion is that this short-circuits **before** any backend
    // call — if it didn't, the tool would otherwise try to reach
    // `staging-api.tinyhumans.ai` and fail with a network error, which
    // would still surface as an error ToolResult.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    let _home_guard = HomeEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioListToolsTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({}))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        !result.is_error,
        "direct-mode list_tools should return success+empty, got error: {}",
        error_text(&result)
    );
    let body = result
        .content
        .iter()
        .find_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    // Empty `tools` array.
    assert!(
        body.contains("\"tools\":[]") || body.contains("\"tools\": []"),
        "direct-mode list_tools body should contain an empty tools array: {body}"
    );
}

#[tokio::test]
async fn execute_tool_per_call_factory_means_no_baked_client() {
    // Regression check for the structural fix: `ComposioExecuteTool::new`
    // takes `Arc<Config>` rather than `ComposioClient`, so a user
    // toggling `composio.mode` mid-session is observed on the very next
    // execute. We exercise this by constructing the tool with a
    // *direct*-mode config but no api_key. The factory must fail with
    // the direct-mode key-missing error rather than silently routing
    // through the backend client. Pre-fix, the tool would have held a
    // backend `ComposioClient` and ignored the mode entirely.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()`
    // per call which reads from `~/.openhuman/config.toml` (or the
    // workspace pointed at by `OPENHUMAN_WORKSPACE`). To isolate the
    // test from the dev's real config we hold `TEST_ENV_LOCK`, point
    // `OPENHUMAN_WORKSPACE` at a tempdir, and persist the test's
    // `Config` to that tempdir's `config.toml` before invoking the tool.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    let _home_guard = HomeEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    // No api_key here — direct-mode factory must reject.
    config.save().await.expect("save fake config to disk");

    let tool = ComposioExecuteTool::new(Arc::new(config));
    // Use a read-scoped slug so the scope/sandbox gates don't short-
    // circuit before the dispatch site.
    let result = tool
        .execute(serde_json::json!({ "tool": "GMAIL_FETCH_EMAILS" }))
        .await
        .unwrap();
    assert!(result.is_error, "direct mode without key must error");
    let msg = error_text(&result);
    // Error must mention direct-mode key configuration, NOT a backend
    // session / staging-api artifact.
    assert!(
        msg.contains("direct mode") && msg.contains("api key"),
        "expected direct-mode key error, got: {msg}"
    );
    assert!(
        !msg.contains("staging-api") && !msg.contains("agent-integrations"),
        "must not leak backend-tenant routing artifacts in direct mode: {msg}"
    );
}

#[tokio::test]
async fn list_toolkits_in_direct_mode_returns_empty_without_hitting_backend() {
    // Same shape as `list_tools_in_direct_mode_returns_empty_without_hitting_backend`
    // — verifies the per-call factory routing for `composio_list_toolkits`.
    // Pre-fix this would have called
    // `staging-api.tinyhumans.ai/agent-integrations/composio/toolkits`
    // regardless of mode and surfaced whatever the backend allowlist
    // returned for the tinyhumans tenant.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    let _home_guard = HomeEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioListToolkitsTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({}))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        !result.is_error,
        "direct-mode list_toolkits should return success+empty, got error: {}",
        error_text(&result)
    );
    let body = result
        .content
        .iter()
        .find_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    assert!(
        body.contains("\"toolkits\":[]") || body.contains("\"toolkits\": []"),
        "direct-mode list_toolkits body should contain an empty toolkits array: {body}"
    );
}

#[test]
fn list_connections_in_direct_mode_resolves_to_direct_client_kind() {
    // Verifies the routing property without making a network call:
    // when mode=direct with an inline api_key, create_composio_client
    // returns a Direct variant. The list_connections tool uses the same
    // factory call, so if the factory picks Direct the tool will route
    // to direct_list_connections (not the backend short-circuit).
    // Previously the tool short-circuited to empty-success in direct mode
    // which caused the agent to incorrectly see no connections (#1710).
    let config = direct_mode_config();
    let kind = crate::openhuman::integrations::composio::client::create_composio_client(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT,
        "list_connections tool: factory should pick the direct variant when mode=direct"
    );
}

#[tokio::test]
async fn authorize_in_direct_mode_refuses_with_app_composio_dev_hint() {
    // `composio_authorize` cannot meaningfully proceed in direct mode
    // — the OAuth handoff has to happen through the user's personal
    // Composio account, not the backend's
    // `/agent-integrations/composio/authorize` route. Pre-fix the tool
    // would have silently hit the backend regardless.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    // Also hold the composio cache lock so we don't race against ops_tests
    // that mutate INTEGRATIONS_CACHE at the same time as we reload config.
    let _cache_guard =
        crate::openhuman::integrations::composio::connected_integrations::composio_cache_test_lock(
        );
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());
    let _home_guard = HomeEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioAuthorizeTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({ "toolkit": "gmail" }))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        result.is_error,
        "direct-mode authorize must refuse, got success"
    );
    let msg = error_text(&result);
    assert!(
        msg.contains("direct mode") && msg.contains("app.composio.dev"),
        "expected direct-mode hint to point at app.composio.dev, got: {msg}"
    );
    assert!(
        !msg.contains("staging-api") && !msg.contains("agent-integrations"),
        "must not leak backend-tenant routing artifacts in direct mode: {msg}"
    );
}

// ── composio_connect park bound (issue #4756) ────────────────────────
//
// composio_connect parks on the inline-connect approval card up to the gate's
// full TTL. When nothing resolves it (headless/eval run, or a disconnected
// chat client) that blocked the whole turn to an empty reply, while the read
// path returns a graceful "not connected" prompt fast. The park is now bounded
// by `composio_connect_timeout()`; these cover its pure env parser.

#[test]
fn parse_composio_connect_timeout_defaults_when_absent_or_garbage() {
    let default = std::time::Duration::from_secs(DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS);
    // Absent → default bound (never unbounded by accident).
    assert_eq!(parse_composio_connect_timeout(None), Some(default));
    // Unparseable → default bound.
    assert_eq!(parse_composio_connect_timeout(Some("soon")), Some(default));
    assert_eq!(parse_composio_connect_timeout(Some("")), Some(default));
}

#[test]
fn parse_composio_connect_timeout_honors_override_and_zero_opt_out() {
    // Explicit value → that many seconds.
    assert_eq!(
        parse_composio_connect_timeout(Some("45")),
        Some(std::time::Duration::from_secs(45))
    );
    // Whitespace tolerated.
    assert_eq!(
        parse_composio_connect_timeout(Some("  90 ")),
        Some(std::time::Duration::from_secs(90))
    );
    // `0` → opt out of the composio-side bound (fall back to the gate TTL).
    assert_eq!(parse_composio_connect_timeout(Some("0")), None);
}
