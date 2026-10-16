use super::*;
use axum::routing::{get, post};
use axum::{Json, Router};
use openhuman_embed::__host::config::Config;
use serde_json::json;
use std::sync::Arc;

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[test]
fn from_config_errors_without_a_backend_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    let error = CryptoClient::from_config(&config)
        .err()
        .expect("no credential, no client");
    assert!(error.contains("signed-in session"), "{error}");
}

#[tokio::test]
async fn routes_swap_and_bridge_hit_the_crypto_routes_and_unwrap_the_envelope() {
    let app = Router::new()
        .route(
            "/agent-integrations/crypto/routes",
            get(|| async { Json(json!({ "success": true, "data": { "chains": [1, 10] } })) }),
        )
        .route(
            "/agent-integrations/crypto/swap",
            post(|Json(body): Json<serde_json::Value>| async move {
                Json(json!({ "success": true, "data": { "swap": body } }))
            }),
        )
        .route(
            "/agent-integrations/crypto/bridge",
            post(|Json(body): Json<serde_json::Value>| async move {
                Json(json!({ "success": true, "data": { "bridge": body } }))
            }),
        );
    let base = start_mock_backend(app).await;
    let client = CryptoClient::new(Arc::new(IntegrationClient::new(base, "test-token".into())));

    assert_eq!(client.routes().await.unwrap(), json!({ "chains": [1, 10] }));
    assert_eq!(
        client.swap_tx(&json!({ "chainId": 1 })).await.unwrap(),
        json!({ "swap": { "chainId": 1 } })
    );
    assert_eq!(
        client
            .bridge_tx(&json!({ "srcChainId": 1, "dstChainId": 10 }))
            .await
            .unwrap(),
        json!({ "bridge": { "srcChainId": 1, "dstChainId": 10 } })
    );
}
