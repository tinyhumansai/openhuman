use axum::{routing::post, Json, Router};
use serde_json::{json, Value};

use super::*;
use crate::openhuman::integrations::composio::module_client::module_guard;

/// Regression for #5751: the flow adapter used to parse and log the chosen
/// account, then call `execute_tool` without it. The request therefore ran
/// against the ambient account. Pin the final backend JSON body so that
/// dropping the id at any point in this seam fails the test.
///
/// The seam is longer now — the id crosses the bus into the module before
/// it reaches a request — which is exactly why this still asserts on the
/// body the backend receives rather than on anything in between.
#[tokio::test]
async fn backend_dispatch_forwards_the_workflow_connection_id() {
    let _serialised = module_guard().await;

    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["tool"], "GITHUB_LIST_REPOSITORY_ISSUES");
            assert_eq!(body["arguments"]["owner"], "tinyhumansai");
            assert_eq!(body["connectionId"], "ca_target_account");
            Json(json!({
                "success": true,
                "data": {
                    "data": { "issues": [] },
                    "successful": true,
                    "error": null,
                    "costUsd": 0.0
                }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let addr = listener.local_addr().expect("mock backend address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock backend");
    });

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.api_url = Some(format!("http://{addr}"));
    crate::openhuman::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");

    let response = execute_for_connection(
        &config,
        "GITHUB_LIST_REPOSITORY_ISSUES",
        Some(json!({ "owner": "tinyhumansai" })),
        Some("ca_target_account"),
    )
    .await
    .expect("backend dispatch should succeed");

    assert!(response.successful);
}
