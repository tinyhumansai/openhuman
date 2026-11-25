use super::*;

async fn endpoint() -> (String, tokio::task::JoinHandle<()>) {
    use axum::{routing::post, Json, Router};

    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            Json(serde_json::json!({
                "object": "list",
                "data": [{
                    "object": "embedding",
                    "index": 0,
                    "embedding": [0.1_f32, 0.2, 0.3, 0.4]
                }],
                "model": "bge-m3"
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://127.0.0.1:{}/v1",
        listener.local_addr().unwrap().port()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, server)
}

async fn config_context(provider: String) -> (tempfile::TempDir, DispatchCtx) {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.config_path = tmp.path().join("config.toml");
    config.memory.embedding_provider = provider;
    config.memory.embedding_model = "bge-m3".to_string();
    config.memory.embedding_dimensions = 4;
    config.save().await.unwrap();
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    (tmp, DispatchCtx::new(core))
}

async fn reloaded_config(tmp: &tempfile::TempDir) -> Config {
    Config::load_from_config_path(&tmp.path().join("config.toml"), tmp.path())
        .await
        .unwrap()
}

#[tokio::test]
async fn endpoint_only_custom_change_reports_persisted_provider_signature() {
    let (endpoint, server) = endpoint().await;
    let (tmp, ctx) = config_context("custom:http://127.0.0.1:1/v1".to_string()).await;

    let outcome = update_settings(&ctx, None, None, None, Some(endpoint.clone()), None)
        .await
        .unwrap();
    let persisted = reloaded_config(&tmp).await;
    let expected = format!("provider=custom:{endpoint};model=bge-m3;dims=4");

    assert_eq!(
        persisted.memory.embedding_provider,
        format!("custom:{endpoint}")
    );
    assert_eq!(outcome.value["signature_changed"], true);
    assert_eq!(outcome.value["new_signature"], expected);
    server.abort();
}

#[tokio::test]
async fn explicit_custom_provider_with_same_endpoint_keeps_signature_unchanged() {
    let (endpoint, server) = endpoint().await;
    let (_tmp, ctx) = config_context(format!("custom:{endpoint}")).await;

    let outcome = update_settings(
        &ctx,
        Some("custom".to_string()),
        None,
        None,
        Some(endpoint),
        None,
    )
    .await
    .unwrap();

    assert_eq!(outcome.value["signature_changed"], false);
    server.abort();
}

#[tokio::test]
async fn auto_detected_dimensions_are_used_in_persisted_provider_signature() {
    let (endpoint, server) = endpoint().await;
    let (tmp, ctx) = config_context("custom:http://127.0.0.1:1/v1".to_string()).await;

    let outcome = update_settings(&ctx, None, None, Some(8), Some(endpoint.clone()), None)
        .await
        .unwrap();
    let persisted = reloaded_config(&tmp).await;
    let expected = format!("provider=custom:{endpoint};model=bge-m3;dims=4");

    assert_eq!(
        persisted.memory.embedding_provider,
        format!("custom:{endpoint}")
    );
    assert_eq!(persisted.memory.embedding_dimensions, 4);
    assert_eq!(outcome.value["signature_changed"], true);
    assert_eq!(outcome.value["new_signature"], expected);
    server.abort();
}

#[tokio::test]
async fn custom_verification_keeps_model_guidance_and_rejects_the_save() {
    use axum::{
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };

    let app = Router::new()
        .route(
            "/v1/embeddings",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "message": "model does not support embeddings" }
                    })),
                )
            }),
        )
        .route(
            "/v1/models",
            get(|| async {
                Json(serde_json::json!({
                    "data": [{ "id": "text-embedding-bge-m3" }]
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://127.0.0.1:{}/v1",
        listener.local_addr().unwrap().port()
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (tmp, ctx) = config_context("custom:http://127.0.0.1:1/v1".to_string()).await;

    let outcome = update_settings(
        &ctx,
        Some("custom".to_string()),
        Some("bge-m3".to_string()),
        Some(8),
        Some(endpoint),
        None,
    )
    .await
    .unwrap();
    let persisted = reloaded_config(&tmp).await;

    assert_eq!(outcome.value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
    assert_eq!(outcome.value["requested_model"], "bge-m3");
    assert_eq!(outcome.value["suggested_model"], "text-embedding-bge-m3");
    assert_eq!(
        persisted.memory.embedding_provider,
        "custom:http://127.0.0.1:1/v1"
    );
    assert_eq!(persisted.memory.embedding_dimensions, 4);
    server.abort();
}

#[tokio::test]
async fn settings_reports_native_effective_embedder_resolution() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    config.embeddings_provider = Some("none".to_string());
    assert_eq!(
        get_settings(&config).await.unwrap().value["effective_provider"],
        "none"
    );

    config.embeddings_provider = Some("ollama:nomic-embed-text".to_string());
    config.memory.embedding_provider = "openai".to_string();
    assert_eq!(
        get_settings(&config).await.unwrap().value["effective_provider"],
        "ollama"
    );

    config.embeddings_provider = Some("openhuman".to_string());
    config.memory.embedding_provider = "custom:https://example.test/v1".to_string();
    assert_eq!(
        get_settings(&config).await.unwrap().value["effective_provider"],
        "custom"
    );

    config.memory.embedding_provider = "managed".to_string();
    let auth = crate::credentials::AuthService::from_config(&config);
    auth.store_provider_token(
        crate::credentials::APP_SESSION_PROVIDER,
        "default",
        "test-session",
        Default::default(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        get_settings(&config).await.unwrap().value["effective_provider"],
        "cloud"
    );

    auth.remove_profile(crate::credentials::APP_SESSION_PROVIDER, "default")
        .await
        .unwrap();
    assert_eq!(
        get_settings(&config).await.unwrap().value["effective_provider"],
        "unconfigured"
    );
}
