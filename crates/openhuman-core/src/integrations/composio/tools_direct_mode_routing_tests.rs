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
    use crate::integrations::composio::types::ComposioToolsResponse;

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
    let kind = crate::integrations::composio::client::resolve_composio_route(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::config::schema::COMPOSIO_MODE_DIRECT,
        "factory should pick the direct variant when mode=direct"
    );
}

#[test]
fn execute_tool_resolves_to_backend_kind_when_mode_is_backend() {
    // Reverse of the above — confirms the backend path still wins when
    // the user is on default (mode = "backend") and a session token is
    // present. Without the token, `resolve_composio_route` returns
    // Err("no backend session"); store one to get past that gate.
    crate::integrations::composio::test_backend_route::register();
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::security::credentials::APP_SESSION_PROVIDER,
            crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    let kind = crate::integrations::composio::client::resolve_composio_route(&config)
        .expect("backend mode with session token must resolve");
    assert_eq!(
        kind.mode(),
        crate::config::schema::COMPOSIO_MODE_BACKEND,
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
    use crate::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    let _home_guard = EnvVarGuard::set("HOME", tmp.path());

    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
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
            tinytools::ToolContent::Text { text } => Some(text.clone()),
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
    use crate::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().unwrap();
    let _workspace_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    let _home_guard = EnvVarGuard::set("HOME", tmp.path());

    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
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
    use crate::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    let _home_guard = EnvVarGuard::set("HOME", tmp.path());

    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
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
            tinytools::ToolContent::Text { text } => Some(text.clone()),
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
    // when mode=direct with an inline api_key, resolve_composio_route
    // returns a Direct variant. The list_connections tool uses the same
    // factory call, so if the factory picks Direct the tool will route
    // to direct_list_connections (not the backend short-circuit).
    // Previously the tool short-circuited to empty-success in direct mode
    // which caused the agent to incorrectly see no connections (#1710).
    let config = direct_mode_config();
    let kind = crate::integrations::composio::client::resolve_composio_route(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::config::schema::COMPOSIO_MODE_DIRECT,
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
    use crate::config::TEST_ENV_LOCK;
    // Also hold the composio cache lock so we don't race against ops_tests
    // that mutate INTEGRATIONS_CACHE at the same time as we reload config.
    let _cache_guard =
        crate::integrations::composio::connected_integrations::composio_cache_test_lock_async()
            .await;
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    let _home_guard = EnvVarGuard::set("HOME", tmp.path());

    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
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
fn composio_connect_outlives_its_approval_park() {
    // The per-tool deadline must not race the in-execute approval park: the
    // tool's budget is the park bound plus slack, and unbounded when the
    // operator opted out of the bound.
    let bound = std::time::Duration::from_secs(DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS);
    match composio_connect_tool_timeout(Some(bound)) {
        tinytools::ToolTimeout::Millis(ms) => {
            assert!(
                ms > bound.as_millis() as u64,
                "budget {ms}ms must exceed the park bound"
            );
        }
        other => panic!("expected an explicit budget, got {other:?}"),
    }
    assert_eq!(
        composio_connect_tool_timeout(None),
        tinytools::ToolTimeout::Unbounded
    );
}

#[test]
fn composio_connect_timeout_saturates_instead_of_overflowing() {
    // `u64::MAX` seconds plus the slack must not panic in `Duration` addition.
    let bound = std::time::Duration::from_secs(u64::MAX);
    assert_eq!(
        composio_connect_tool_timeout(Some(bound)),
        tinytools::ToolTimeout::Millis(u64::MAX)
    );
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

#[test]
fn composio_connect_charges_pre_gate_work_to_the_park_bound() {
    use std::time::Duration;
    let bound = Duration::from_secs(DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS);
    let tool_budget = match composio_connect_tool_timeout(Some(bound)) {
        tinytools::ToolTimeout::Millis(ms) => Duration::from_millis(ms),
        other => panic!("expected an explicit budget, got {other:?}"),
    };
    // 50 s of slow config/connection/catalog reads before the card is raised:
    // pre-gate + park must still end before the tool deadline, with the
    // remaining slack left for the post-approval liveness check.
    let pre_gate = Duration::from_secs(50);
    let park = remaining_park_bound(Some(bound), pre_gate).unwrap();
    assert_eq!(park, bound - pre_gate);
    assert!(pre_gate + park < tool_budget);

    // Pre-gate work that used the whole bound still raises a short park.
    assert_eq!(
        remaining_park_bound(Some(bound), bound * 2),
        Some(Duration::from_secs(1))
    );
    // Opting out of the bound stays unbounded.
    assert_eq!(remaining_park_bound(None, pre_gate), None);
}

/// The list tool answers the same through the run's context carrier as it does
/// without one: the scope-pref migration it triggers runs for the context of
/// the run that called the tool, not whichever scope polls it.
#[tokio::test]
async fn list_tools_with_a_run_context_carrier_matches_the_bare_call() {
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::config::TEST_ENV_LOCK;
    use crate::core::runtime::{CoreContext, DomainSet};
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    let _home_guard = EnvVarGuard::set("HOME", tmp.path());

    let mut config = crate::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("create workspace dir");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let run = OpenHumanRunContext::new()
        .with_core(Some(CoreContext::for_test(DomainSet::full(), None)))
        .into_tinyagents(tinyagents_harness::context::RunConfig::new(
            "list-tools-carrier",
        ));
    let carrier = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("list-tools-carrier-call"),
    );

    let tool = ComposioListToolsTool::new(Arc::new(config));
    let bare = tool.execute(serde_json::json!({})).await.unwrap();
    let carried = tool
        .execute_with_context(
            serde_json::json!({}),
            tinytools::ToolCallOptions::default(),
            Some(&carrier),
        )
        .await
        .unwrap();
    assert!(!carried.is_error, "{}", error_text(&carried));
    assert_eq!(bare.is_error, carried.is_error);
    assert_eq!(bare.output(), carried.output());
}
