use super::*;
use std::collections::HashMap;
// Production code now routes managed construction through
// `create_embedding_provider_with_config`; this low-level custom-endpoint
// regression test still drives the credentialed factory directly.
use super::super::create_embedding_provider_with_credentials;
use tempfile::TempDir;

/// The seam the memory factory depends on (TAURI-RUST-52S fix): the three
/// `create_memory_with_local_ai` call sites resolve the user's stored BYO
/// embedding credential via `resolve_api_key` and thread it into the
/// provider. If this lookup silently returns "" for a configured key —
/// wrong cred slug, encryption mismatch, profile-store regression — the
/// memory pipeline reverts to sending an empty bearer and Cohere 401s on
/// every embed. Lock the round-trip: store under `embeddings:<slug>`, read
/// it back; an unrelated provider must stay empty (no cross-bleed).
#[tokio::test]
async fn resolve_api_key_returns_stored_embeddings_credential() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    // Nothing stored yet → empty (the empty-key guard's "" input).
    assert_eq!(resolve_api_key(&config, "cohere").await, "");

    // Store a Cohere embeddings key exactly as `set_api_key` does.
    AuthService::from_config(&config)
        .store_provider_token(
            "embeddings:cohere",
            "default",
            "sk-cohere-test",
            HashMap::new(),
            true,
        )
        .await
        .unwrap();

    // Resolve returns it; a provider with no stored key stays empty.
    assert_eq!(resolve_api_key(&config, "cohere").await, "sk-cohere-test");
    assert_eq!(resolve_api_key(&config, "voyage").await, "");
}

#[tokio::test]
async fn custom_profile_is_retained_and_returned_while_embeddings_are_disabled() {
    let mut config = Config::default();
    config.memory.embedding_provider = "custom:https://embeddings.example.com/v1".to_string();
    config.memory.embedding_model = "acme-embed-v2".to_string();
    config.memory.embedding_dimensions = 2048;

    remember_active_custom_profile(&mut config);
    config.memory.embedding_provider = "none".to_string();
    config.embeddings_provider = Some("none".to_string());

    let out = get_settings(&config)
        .await
        .expect("get_settings must return the retained Custom profile");
    assert_eq!(out.value["provider"], "none");
    assert_eq!(
        out.value["custom_settings"],
        serde_json::json!({
            "endpoint": "https://embeddings.example.com/v1",
            "model": "acme-embed-v2",
            "dimensions": 2048,
        })
    );
}

/// `custom:<url>` providers must look up under the `embeddings:custom`
/// slug (the inline URL is not part of the credential key), mirroring the
/// slug normalization in `embed`/`set_api_key`.
#[tokio::test]
async fn resolve_api_key_normalizes_custom_prefix_to_custom_slug() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    AuthService::from_config(&config)
        .store_provider_token(
            "embeddings:custom",
            "default",
            "sk-custom-test",
            HashMap::new(),
            true,
        )
        .await
        .unwrap();

    assert_eq!(
        resolve_api_key(&config, "custom:http://localhost:1234").await,
        "sk-custom-test"
    );
}

/// Issue #5017 regression — the request the app sends is correct: a conformant
/// OpenAI-compatible `POST /v1/embeddings` host (right path, the user's model,
/// Bearer key, `{"input":[…],"model":…}` body) verifies successfully. Builds
/// the live custom provider with the endpoint's known width and drives it
/// against a mock that echoes the OpenAI embeddings wire shape, asserting the
/// captured request and a usable returned vector.
#[tokio::test]
async fn conformant_custom_endpoint_verifies_and_sends_expected_request() {
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };

    #[derive(Clone, Default)]
    struct Captured {
        auth: Arc<Mutex<Option<String>>>,
        body: Arc<Mutex<Option<serde_json::Value>>>,
    }

    let captured = Captured::default();
    let app = Router::new()
        .route(
            "/v1/embeddings",
            post(
                |State(cap): State<Captured>,
                 headers: axum::http::HeaderMap,
                 Json(body): Json<serde_json::Value>| async move {
                    *cap.auth.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    *cap.body.lock().unwrap() = Some(body);
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{
                            "object": "embedding",
                            "index": 0,
                            "embedding": [0.1_f32, 0.2, 0.3, 0.4]
                        }],
                        "model": "my-embed",
                    }))
                },
            ),
        )
        .route(
            "/v1/models",
            get(|| async { Json(serde_json::json!({"data": []})) }),
        )
        .with_state(captured.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://127.0.0.1:{}/v1",
        listener.local_addr().unwrap().port()
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The live provider validates returned vectors against its configured width.
    // Use the mock endpoint's known width; setup-time custom probing discovers
    // that width separately before saving the live configuration.
    let embedder = create_embedding_provider_with_credentials(
        "custom",
        "gpt-5-mini",
        4,
        "sk-secret-key",
        Some(&base),
    )
    .await
    .expect("provider builds");

    let vectors = embedder
        .embed(&["connection test".to_string()])
        .await
        .expect("conformant endpoint must verify");
    let probe_dims = vectors.first().map(|v| v.len()).unwrap_or(0);
    assert_eq!(
        probe_dims, 4,
        "auto-detected the endpoint's real vector length"
    );

    assert_eq!(vectors.first().map(Vec::len), Some(4));

    // The exact request: user's model forwarded + Bearer key present.
    assert_eq!(
        captured.auth.lock().unwrap().as_deref(),
        Some("Bearer sk-secret-key"),
        "API key must be sent on the test-embed request"
    );
    let body = captured
        .body
        .lock()
        .unwrap()
        .clone()
        .expect("body captured");
    assert_eq!(
        body["model"], "gpt-5-mini",
        "user-supplied model must be forwarded"
    );
    assert_eq!(
        body["input"],
        serde_json::json!(["connection test"]),
        "input is the OpenAI array-of-strings shape"
    );
}

/// #5356: the managed paths build the cloud embedder through the
/// config-aware factory, so the bearer resolver reads the config-scoped
/// credential store. With no stored `app-session` token the resolver
/// short-circuits to the backend-session error *before* any network call
/// (privacy defaults to `Standard`, so egress is allowed and the failure is
/// the missing session, not a local-only block). Also covers the rerouted
/// `test_connection` construction line.
#[tokio::test]
async fn test_connection_managed_without_provider_reports_backend_unavailable() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    let out = test_connection(&config, Some("managed"), Some("voyage-3-large"), Some(1024))
        .await
        .expect("rpc returns Ok carrying a success flag");
    assert_eq!(
        out.value["success"],
        serde_json::json!(false),
        "managed test with no session must not pass"
    );
    let err = out.value["error"].as_str().unwrap_or_default();
    assert!(
        err.contains(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "managed test with no provider must report BACKEND_UNAVAILABLE, got: {err}"
    );
}

#[tokio::test]
async fn test_connection_returns_model_construction_failures_as_rpc_errors() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    let error = test_connection(&config, Some("not-a-provider"), Some("model"), Some(4))
        .await
        .expect_err("invalid provider construction must remain an RPC error");
    assert!(error.contains("unknown embedding provider"), "{error}");
}

#[tokio::test]
async fn test_connection_reports_embedding_failures_in_the_failure_payload() {
    use axum::{http::StatusCode, routing::post, Router};

    let app = Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "embedding unavailable") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let _guard = crate::inference::inference_test_guard_async().await;
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.local_ai.base_url = Some(base_url);

    let outcome = test_connection(&config, Some("ollama"), Some("model"), Some(4))
        .await
        .expect("model construction succeeded, so the failure stays in the outcome");
    assert_eq!(outcome.value["success"], false);
    assert!(outcome.value["error"]
        .as_str()
        .unwrap()
        .contains("embedding"));
    server.abort();
}

/// Live `embed` (RPC) for managed also routes through the config-aware
/// factory; with no session it surfaces the same backend-session error.
#[tokio::test]
async fn embed_managed_without_provider_errors_with_backend_unavailable() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.memory.embedding_provider = "managed".to_string();
    config.memory.embedding_model = "voyage-3-large".to_string();
    let err = embed(&config, &["hello".to_string()])
        .await
        .expect_err("managed embed with no session must error");
    assert!(
        err.contains(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "expected BACKEND_UNAVAILABLE, got: {err}"
    );
}

/// `provider_from_config` (reused by other domains for direct embedding)
/// routes managed construction through the config-aware factory too — pure
/// construction, so it builds the cloud provider without a network call.
#[tokio::test]
async fn provider_from_config_managed_builds_cloud() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.memory.embedding_provider = "managed".to_string();
    config.memory.embedding_model = "voyage-3-large".to_string();
    config.memory.embedding_dimensions = 1024;
    let provider = provider_from_config(&config)
        .await
        .expect("managed provider must build");
    assert_eq!(provider.name(), "cloud");
}
