use super::*;

fn unsigned_in_config() -> Config {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    std::mem::forget(tmp);
    config
}

fn direct_mode_no_key_config() -> Config {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = tinymemory_api::host::COMPOSIO_MODE_DIRECT.to_string();
    std::mem::forget(tmp);
    config
}

#[tokio::test]
async fn list_slack_connections_errors_with_slack_ingest_prefix_when_no_credentials() {
    // Pre-Option-C `sync_trigger_rpc` / `sync_status_rpc` returned
    // the literal string "[slack_ingest] Composio client unavailable
    // (user not signed in?)" because the gate was
    // `build_composio_client(...).is_none()`. Post-Option-C the
    // gate is the factory, so the error surfaces the *factory's*
    // "no backend session" message wrapped with the domain prefix.
    // We exercise the shared helper directly so the test doesn't
    // depend on the SlackProvider being registered in the test
    // global registry (that registration is a runtime concern
    // owned by `init_default_providers`, not relevant to the
    // factory wiring under test here).
    let config = unsigned_in_config();
    let err = list_slack_connections(&config).await.unwrap_err();
    assert!(
        err.starts_with("[slack_ingest] list_connections:"),
        "factory-routed error should keep the [slack_ingest] domain prefix, got: {err}"
    );
    assert!(
        err.contains("no backend session"),
        "backend-mode failure path should surface the factory's session-missing message, \
         got: {err}"
    );
}

#[tokio::test]
async fn list_slack_connections_in_direct_mode_without_api_key_surfaces_direct_mode_error() {
    // Confirms the factory is exercised in direct mode too — when
    // mode=direct but no api_key is stored, the error message
    // surfaces the direct-mode key-missing hint, not the backend
    // session message. Pre-Option-C this returned the backend-only
    // "user not signed in?" message regardless of mode.
    let config = direct_mode_no_key_config();
    let err = list_slack_connections(&config).await.unwrap_err();
    assert!(
        err.starts_with("[slack_ingest] list_connections:"),
        "domain prefix preserved through the factory route, got: {err}"
    );
    assert!(
        err.contains("direct mode") || err.contains("api key"),
        "direct-mode key-missing should surface the direct-mode-specific hint, got: {err}"
    );
}

#[tokio::test]
async fn list_slack_connections_resolves_direct_variant_when_mode_is_direct() {
    // Pin the factory routing: with a direct-mode config + inline
    // api_key, `list_slack_connections` must reach
    // `direct_list_connections` (which then attempts a network
    // call). We can't assert the success path without a mock
    // backend.composio.dev, but we *can* assert the error message
    // identifies the direct arm — proving the factory picked the
    // right branch.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = tinymemory_api::host::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    std::mem::forget(tmp);

    let result = list_slack_connections(&config).await;
    // The network call will fail (test environment has no upstream
    // mock). We only care that the failure label says "direct" —
    // that's the load-bearing evidence the factory routed through
    // the new branch instead of the old backend-only path.
    if let Err(err) = result {
        assert!(
            err.contains("(direct)") || err.contains("direct"),
            "factory must route to the direct arm for mode=direct configs, got: {err}"
        );
    }
    // If the network call somehow succeeds (e.g. CI gateway returns
    // a valid empty envelope), that's also acceptable — the
    // factory still routed correctly.
}

// ── the status surface's degraded shape ─────────────────────────────────────
//
// `sync_status_rpc` does two things nothing asserted: it filters the connection
// list to slack rows that are ACTIVE, and it answers a fixed zero-value shape
// because per-connection sync detail is no longer readable — the connector
// module keeps its cursor internally.
//
// The deleted `slack_sync_status_rpc_reports_the_degraded_zero_value_shape`
// covered exactly this, in `tests/raw_coverage/memory_sync_tree_round21_*`,
// which went with the engine (#6161) although the assertion was never about the
// engine. Restored here against a local mock backend (#6172).

#[tokio::test]
async fn status_filters_to_active_slack_and_reports_the_degraded_zero_value_shape() {
    use crate::openhuman::security::credentials::{
        AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agent-integrations/composio/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": { "connections": [
                // The one row that must survive both filters.
                { "id": "conn-slack-active",  "toolkit": "slack", "status": "ACTIVE" },
                // Dropped by the status filter…
                { "id": "conn-slack-pending", "toolkit": "slack", "status": "PENDING" },
                // …and this one by the toolkit filter.
                { "id": "conn-gmail-active",  "toolkit": "gmail", "status": "ACTIVE" },
            ] }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.api_url = Some(server.uri());
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");

    let outcome = sync_status_rpc(&config, SyncStatusRequest::default())
        .await
        .expect("status rpc");

    // ── the filter ──────────────────────────────────────────────────────────
    assert_eq!(
        outcome.value.connections.len(),
        1,
        "only the ACTIVE slack connection qualifies; got {:?}",
        outcome.value.connections
    );
    let row = &outcome.value.connections[0];
    assert_eq!(row.connection_id, "conn-slack-active");

    // ── the degraded shape ──────────────────────────────────────────────────
    //
    // Four fixed zero values, asserted individually rather than as a struct
    // comparison: each one is a separate promise to the status table, and a
    // struct literal would hide which of them a future change broke.
    assert_eq!(row.per_channel_cursors, "{}");
    assert_eq!(row.synced_ids_count, 0);
    assert_eq!(row.requests_used_today, 0);
    assert_eq!(row.daily_request_limit, 0);

    // ── and the log that explains it ────────────────────────────────────────
    //
    // The zeros are indistinguishable from "a connection that has synced
    // nothing yet", so the log line is what tells an operator the detail is
    // gone rather than empty. Without it the shape above is a silent lie.
    assert!(
        outcome
            .logs
            .iter()
            .any(|line| line.contains("connections=1") && line.contains("no longer available")),
        "the status log must explain the degraded read: {:?}",
        outcome.logs
    );
}
