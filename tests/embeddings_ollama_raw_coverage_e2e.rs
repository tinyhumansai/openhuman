//! Raw-line oriented E2E coverage for the Ollama embedding provider.
//!
//! These tests use the public embedding provider API against a local mock
//! Ollama HTTP server. They avoid a real daemon while exercising the same
//! request, validation, and NaN-recovery branches used in production.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};

use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::embeddings::catalog;
use openhuman_core::openhuman::embeddings::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use openhuman_core::openhuman::embeddings::cohere::CohereEmbedding;
use openhuman_core::openhuman::embeddings::noop::NoopEmbedding;
use openhuman_core::openhuman::embeddings::ollama::DEFAULT_OLLAMA_URL;
use openhuman_core::openhuman::embeddings::openai::OpenAiEmbedding;
use openhuman_core::openhuman::embeddings::retry_after::{
    backoff_ms_for_attempt, parse_retry_after_ms, BASE_BACKOFF_MS, MAX_BACKOFF_MS,
};
use openhuman_core::openhuman::embeddings::voyage::VoyageEmbedding;
use openhuman_core::openhuman::embeddings::{
    create_embedding_provider, create_embedding_provider_with_credentials, EmbeddingProvider,
    OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL,
};

async fn serve_mock_ollama(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock ollama");
    let addr: SocketAddr = listener.local_addr().expect("mock ollama addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock ollama serve");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[derive(Clone, Copy)]
enum OpenAiMockBehavior {
    RetryThenSuccess,
    CountMismatch,
    BadEmbeddingItem,
    MissingEmbedding,
    DimensionMismatch,
    MissingData,
    Non2xx,
}

#[derive(Clone)]
struct OpenAiMockState {
    behavior: OpenAiMockBehavior,
    attempts: Arc<Mutex<usize>>,
    requests: Arc<Mutex<Vec<Value>>>,
    auth_headers: Arc<Mutex<Vec<Option<String>>>>,
}

impl OpenAiMockState {
    fn new(behavior: OpenAiMockBehavior) -> Self {
        Self {
            behavior,
            attempts: Arc::new(Mutex::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
            auth_headers: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

async fn serve_mock_openai(behavior: OpenAiMockBehavior) -> (String, OpenAiMockState) {
    let state = OpenAiMockState::new(behavior);
    let app = Router::new()
        .route("/v1/embeddings", post(mock_openai_handler))
        .route("/openai/v1/embeddings", post(mock_openai_handler))
        .route("/api/v2/embeddings", post(mock_openai_handler))
        .route("/embeddings", post(mock_openai_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock openai");
    let addr = listener.local_addr().expect("mock openai addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock openai serve");
    });
    (format!("http://127.0.0.1:{}", addr.port()), state)
}

async fn serve_mock_cohere(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock cohere");
    let addr = listener.local_addr().expect("mock cohere addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock cohere serve");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn record_openai_request(state: &OpenAiMockState, headers: &HeaderMap, body: Value) -> usize {
    let mut attempts = state.attempts.lock().expect("attempts lock");
    *attempts += 1;
    let attempt = *attempts;
    drop(attempts);

    state.requests.lock().expect("requests lock").push(body);
    state.auth_headers.lock().expect("auth headers lock").push(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
    );
    attempt
}

async fn mock_openai_handler(
    State(state): State<OpenAiMockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let attempt = record_openai_request(&state, &headers, body);

    match state.behavior {
        OpenAiMockBehavior::RetryThenSuccess if attempt == 1 => (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, "0")],
            "slow down",
        )
            .into_response(),
        OpenAiMockBehavior::RetryThenSuccess => Json(json!({
            "data": [
                { "embedding": [1.0, 2.0] },
                { "embedding": [3.0, 4.0] }
            ]
        }))
        .into_response(),
        OpenAiMockBehavior::CountMismatch => {
            Json(json!({ "data": [{ "embedding": [1.0, 2.0] }] })).into_response()
        }
        OpenAiMockBehavior::BadEmbeddingItem => {
            Json(json!({ "data": [{ "embedding": [1.0, "bad"] }] })).into_response()
        }
        OpenAiMockBehavior::MissingEmbedding => Json(json!({ "data": [{}] })).into_response(),
        OpenAiMockBehavior::DimensionMismatch => {
            Json(json!({ "data": [{ "embedding": [1.0, 2.0, 3.0] }] })).into_response()
        }
        OpenAiMockBehavior::MissingData => Json(json!({ "not_data": [] })).into_response(),
        OpenAiMockBehavior::Non2xx => {
            (StatusCode::BAD_REQUEST, "bad embedding request").into_response()
        }
    }
}

#[tokio::test]
async fn openai_embed_retries_and_round_trips_auth_body_and_vectors() {
    let (base_url, state) = serve_mock_openai(OpenAiMockBehavior::RetryThenSuccess).await;
    let provider = OpenAiEmbedding::new(&base_url, "test-key", "mock-openai", 2);

    assert_eq!(provider.name(), "openai");
    assert_eq!(provider.model_id(), "mock-openai");
    assert_eq!(provider.dimensions(), 2);
    assert_eq!(provider.base_url(), base_url);
    assert_eq!(provider.model(), "mock-openai");
    assert_eq!(
        provider.embeddings_url(),
        format!("{base_url}/v1/embeddings")
    );

    let vectors = provider
        .embed(&["first", "second"])
        .await
        .expect("openai retry success");

    assert_eq!(vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    assert_eq!(*state.attempts.lock().expect("attempts lock"), 2);

    let auth_headers = state.auth_headers.lock().expect("auth headers lock");
    assert_eq!(
        auth_headers.as_slice(),
        [
            Some("Bearer test-key".to_string()),
            Some("Bearer test-key".to_string())
        ]
    );
    drop(auth_headers);

    let requests = state.requests.lock().expect("requests lock");
    assert_eq!(
        requests[0].get("model").and_then(Value::as_str),
        Some("mock-openai")
    );
    assert_eq!(
        requests[0].pointer("/input/0").and_then(Value::as_str),
        Some("first")
    );
    assert_eq!(
        requests[0].pointer("/input/1").and_then(Value::as_str),
        Some("second")
    );
}

#[tokio::test]
async fn openai_embed_handles_explicit_paths_empty_inputs_and_missing_auth() {
    let (base_url, state) = serve_mock_openai(OpenAiMockBehavior::RetryThenSuccess).await;
    let api_provider = OpenAiEmbedding::new(&format!("{base_url}/api/v2"), "", "mock-path", 2);
    assert_eq!(
        api_provider.embeddings_url(),
        format!("{base_url}/api/v2/embeddings")
    );

    let vectors = api_provider
        .embed(&["first", "second"])
        .await
        .expect("explicit api path success");
    assert_eq!(vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    assert_eq!(
        state.auth_headers.lock().expect("auth headers lock").last(),
        Some(&None)
    );

    let endpoint_provider = OpenAiEmbedding::new(&format!("{base_url}/embeddings"), "", "m", 2);
    assert_eq!(
        endpoint_provider.embeddings_url(),
        format!("{base_url}/embeddings")
    );
    assert_eq!(
        endpoint_provider.embed(&[]).await.expect("empty input"),
        Vec::<Vec<f32>>::new()
    );

    let invalid_url_provider = OpenAiEmbedding::new("not-a-url", "", "m", 0);
    assert_eq!(
        invalid_url_provider.embeddings_url(),
        "not-a-url/v1/embeddings"
    );
}

#[tokio::test]
async fn openai_embed_reports_response_validation_and_http_errors() {
    let (count_url, _) = serve_mock_openai(OpenAiMockBehavior::CountMismatch).await;
    let count_provider = OpenAiEmbedding::new(&count_url, "k", "m", 2);
    assert!(count_provider
        .embed(&["a", "b"])
        .await
        .expect_err("count mismatch")
        .to_string()
        .contains("count mismatch"));

    let (bad_item_url, _) = serve_mock_openai(OpenAiMockBehavior::BadEmbeddingItem).await;
    let bad_item_provider = OpenAiEmbedding::new(&bad_item_url, "k", "m", 2);
    assert!(bad_item_provider
        .embed(&["a"])
        .await
        .expect_err("non numeric")
        .to_string()
        .contains("non-numeric"));

    let (missing_item_url, _) = serve_mock_openai(OpenAiMockBehavior::MissingEmbedding).await;
    let missing_item_provider = OpenAiEmbedding::new(&missing_item_url, "k", "m", 2);
    assert!(missing_item_provider
        .embed(&["a"])
        .await
        .expect_err("missing embedding")
        .to_string()
        .contains("missing 'embedding'"));

    let (dim_url, _) = serve_mock_openai(OpenAiMockBehavior::DimensionMismatch).await;
    let dim_provider = OpenAiEmbedding::new(&dim_url, "k", "m", 2);
    assert!(dim_provider
        .embed(&["a"])
        .await
        .expect_err("dimension mismatch")
        .to_string()
        .contains("dimension mismatch"));

    let (missing_url, _) = serve_mock_openai(OpenAiMockBehavior::MissingData).await;
    let missing_provider = OpenAiEmbedding::new(&missing_url, "k", "m", 2);
    assert!(missing_provider
        .embed(&["a"])
        .await
        .expect_err("missing data")
        .to_string()
        .contains("missing 'data'"));

    let (non_2xx_url, _) = serve_mock_openai(OpenAiMockBehavior::Non2xx).await;
    let non_2xx_provider = OpenAiEmbedding::new(&non_2xx_url, "k", "m", 2);
    assert!(non_2xx_provider
        .embed(&["a"])
        .await
        .expect_err("http error")
        .to_string()
        .contains("Embedding API error"));
}

#[tokio::test]
async fn cloud_embedding_uses_seeded_session_token_and_reports_missing_auth() {
    let (base_url, state) = serve_mock_openai(OpenAiMockBehavior::RetryThenSuccess).await;
    let state_dir = tempfile::tempdir().expect("tempdir");
    AuthService::new(state_dir.path(), false)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "cloud-session-token",
            Default::default(),
            true,
        )
        .expect("seed cloud auth");

    let provider = OpenHumanCloudEmbedding::new(
        Some(format!("{base_url}/")),
        Some(state_dir.path().to_path_buf()),
        false,
        "cloud-model",
        2,
    );
    assert_eq!(provider.name(), "cloud");
    assert_eq!(provider.model_id(), "cloud-model");
    assert_eq!(provider.dimensions(), 2);

    let vectors = provider
        .embed(&["first", "second"])
        .await
        .expect("cloud embed");
    assert_eq!(vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    assert_eq!(
        state.auth_headers.lock().expect("auth headers lock").last(),
        Some(&Some("Bearer cloud-session-token".to_string()))
    );

    let missing_auth = OpenHumanCloudEmbedding::new(
        Some(base_url),
        Some(
            tempfile::tempdir()
                .expect("missing auth dir")
                .path()
                .to_path_buf(),
        ),
        false,
        "cloud-model",
        2,
    );
    assert!(missing_auth
        .embed(&["needs-auth"])
        .await
        .expect_err("missing backend session")
        .to_string()
        .contains("No backend session for cloud embeddings"));
}

#[tokio::test]
async fn cohere_and_voyage_embedding_paths_use_local_compatible_mocks() {
    let cohere_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let cohere_auth = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let cohere_requests_for_route = cohere_requests.clone();
    let cohere_auth_for_route = cohere_auth.clone();
    let cohere_url = serve_mock_cohere(Router::new().route(
        "/v2/embed",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let cohere_requests = cohere_requests_for_route.clone();
            let cohere_auth = cohere_auth_for_route.clone();
            async move {
                cohere_requests.lock().expect("cohere requests").push(body);
                cohere_auth.lock().expect("cohere auth").push(
                    headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned),
                );
                Json(json!({ "embeddings": { "float": [[0.1, 0.2], [0.3, 0.4]] } }))
            }
        }),
    ))
    .await;

    let cohere =
        CohereEmbedding::new("cohere-key", "embed-multilingual-v3.0", 2).with_base_url(cohere_url);
    assert_eq!(cohere.name(), "cohere");
    assert_eq!(cohere.model_id(), "embed-multilingual-v3.0");
    assert_eq!(cohere.dimensions(), 2);
    assert_eq!(
        cohere
            .embed(&["alpha", "beta"])
            .await
            .expect("cohere embed"),
        vec![vec![0.1, 0.2], vec![0.3, 0.4]]
    );
    assert_eq!(
        cohere_auth.lock().expect("cohere auth").as_slice(),
        [Some("Bearer cohere-key".to_string())]
    );
    assert_eq!(
        cohere_requests.lock().expect("cohere requests")[0].pointer("/texts/0"),
        Some(&json!("alpha"))
    );

    let (voyage_url, voyage_state) = serve_mock_openai(OpenAiMockBehavior::RetryThenSuccess).await;
    let voyage = VoyageEmbedding::new_with_base_url("voyage-key", "", 2, &voyage_url);
    assert_eq!(voyage.name(), "voyage");
    assert_eq!(voyage.model_id(), "voyage-3-large");
    assert_eq!(voyage.dimensions(), 2);
    assert_eq!(
        voyage
            .embed(&["first", "second"])
            .await
            .expect("voyage embed"),
        vec![vec![1.0, 2.0], vec![3.0, 4.0]]
    );
    assert!(voyage_state
        .auth_headers
        .lock()
        .expect("voyage auth")
        .iter()
        .any(|header| header.as_deref() == Some("Bearer voyage-key")));
}

#[tokio::test]
async fn cohere_embedding_reports_parse_count_dimension_and_http_errors() {
    let count_url = serve_mock_cohere(Router::new().route(
        "/v2/embed",
        post(|| async { Json(json!({ "embeddings": { "float": [[1.0, 2.0]] } })) }),
    ))
    .await;
    let count_provider = CohereEmbedding::new("k", "m", 2).with_base_url(count_url);
    assert!(count_provider
        .embed(&["a", "b"])
        .await
        .expect_err("cohere count mismatch")
        .to_string()
        .contains("count mismatch"));

    let dim_url = serve_mock_cohere(Router::new().route(
        "/v2/embed",
        post(|| async { Json(json!({ "embeddings": { "float": [[1.0, 2.0, 3.0]] } })) }),
    ))
    .await;
    let dim_provider = CohereEmbedding::new("k", "m", 2).with_base_url(dim_url);
    assert!(dim_provider
        .embed(&["a"])
        .await
        .expect_err("cohere dimension mismatch")
        .to_string()
        .contains("dimension mismatch"));

    let malformed_url = serve_mock_cohere(
        Router::new().route("/v2/embed", post(|| async { (StatusCode::OK, "not-json") })),
    )
    .await;
    let malformed_provider = CohereEmbedding::new("k", "m", 2).with_base_url(malformed_url);
    assert!(malformed_provider
        .embed(&["a"])
        .await
        .expect_err("cohere parse")
        .to_string()
        .contains("parse failed"));

    let non_2xx_url = serve_mock_cohere(Router::new().route(
        "/v2/embed",
        post(|| async { (StatusCode::BAD_REQUEST, "bad cohere request") }),
    ))
    .await;
    let non_2xx_provider = CohereEmbedding::new("k", "m", 2).with_base_url(non_2xx_url);
    assert!(non_2xx_provider
        .embed(&["a"])
        .await
        .expect_err("cohere http error")
        .to_string()
        .contains("Cohere embed API error"));
}

#[tokio::test]
async fn embedding_rate_limit_public_paths_cover_disabled_loopback_and_malformed_urls() {
    use openhuman_core::openhuman::embeddings::rate_limit::{
        acquire_embedding_slot, embedding_rate_limit, set_embedding_rate_limit,
    };

    let original = embedding_rate_limit();
    set_embedding_rate_limit(0);
    assert_eq!(embedding_rate_limit(), 0);
    acquire_embedding_slot("https://api.example.invalid/openai/v1").await;

    set_embedding_rate_limit(60_000);
    acquire_embedding_slot("http://localhost:11434").await;
    acquire_embedding_slot("http://[::1]:11434").await;
    acquire_embedding_slot("not-a-url").await;

    set_embedding_rate_limit(original);
}

#[test]
fn ollama_constructor_normalizes_defaults_and_rejects_runtime_misconfiguration() {
    let defaults = OllamaEmbedding::try_new("  ", "  ", 0).expect("default ollama config");
    assert_eq!(defaults.base_url(), DEFAULT_OLLAMA_URL);
    assert_eq!(defaults.model(), DEFAULT_OLLAMA_MODEL);
    assert_eq!(defaults.dimensions(), DEFAULT_OLLAMA_DIMENSIONS);

    let custom = OllamaEmbedding::try_new("http://[::1]:11434/", "  nomic-embed-text  ", 12)
        .expect("custom ollama config");
    assert_eq!(custom.base_url(), "http://[::1]:11434");
    assert_eq!(custom.model(), "nomic-embed-text");
    assert_eq!(
        custom.signature(),
        "provider=ollama;model=nomic-embed-text;dims=12"
    );

    let explicit = OllamaEmbedding::new("http://127.0.0.1:11434", "mock-model", 3);
    assert_eq!(explicit.base_url(), "http://127.0.0.1:11434");
    assert_eq!(explicit.model(), "mock-model");

    let default = OllamaEmbedding::default();
    assert_eq!(default.base_url(), DEFAULT_OLLAMA_URL);
    assert_eq!(default.model(), DEFAULT_OLLAMA_MODEL);

    for bad_url in [
        "ftp://localhost:11434",
        "http://user:pass@localhost:11434",
        "http://localhost:11434/api",
        "http://localhost:11434/v1/chat/completions",
        "http://localhost:11434?debug=true",
        "http://localhost:11434/#fragment",
    ] {
        assert!(
            OllamaEmbedding::try_new(bad_url, "m", 1).is_err(),
            "bad Ollama URL should be rejected: {bad_url}"
        );
    }
    assert!(OllamaEmbedding::try_new("http://localhost:11434", "local-v1", 1).is_err());
}

#[tokio::test]
async fn embedding_catalog_factory_retry_noop_and_cloud_empty_paths_are_reachable() {
    let providers = catalog::all_providers();
    assert!(providers.iter().any(|provider| provider.slug == "managed"));
    assert!(providers.iter().any(|provider| provider.slug == "cohere"));
    assert_eq!(
        catalog::find_provider("openai")
            .expect("openai provider")
            .label,
        "OpenAI"
    );
    assert!(catalog::find_provider("missing").is_none());
    assert_eq!(
        catalog::find_model("voyage", "voyage-3-large")
            .expect("voyage model")
            .default_dimensions,
        1024
    );
    assert!(catalog::find_model("voyage", "missing").is_none());
    assert_eq!(
        catalog::default_model_for("openai")
            .expect("default openai model")
            .id,
        "text-embedding-3-small"
    );
    assert!(catalog::default_model_for("none").is_none());

    assert_eq!(parse_retry_after_ms(Some(" 5 ")), Some(5_000));
    assert_eq!(
        parse_retry_after_ms(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
        Some(0)
    );
    assert_eq!(parse_retry_after_ms(Some("99999")), Some(MAX_BACKOFF_MS));
    assert_eq!(backoff_ms_for_attempt(2, Some("1")), 1_000);
    assert_eq!(backoff_ms_for_attempt(1, None), BASE_BACKOFF_MS * 2);
    assert_eq!(backoff_ms_for_attempt(10, Some("bad")), MAX_BACKOFF_MS);

    let noop = NoopEmbedding;
    assert_eq!(noop.name(), "none");
    assert_eq!(noop.model_id(), "none");
    assert_eq!(noop.dimensions(), 0);
    assert_eq!(noop.signature(), "provider=none;model=none;dims=0");
    assert_eq!(
        noop.embed(&["ignored"]).await.expect("noop embed"),
        Vec::<Vec<f32>>::new()
    );
    assert!(noop
        .embed_one("ignored")
        .await
        .expect_err("noop embed_one")
        .to_string()
        .contains("Empty embedding result"));

    let cloud = OpenHumanCloudEmbedding::new(
        Some("https://api.example.test/".to_string()),
        None,
        false,
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    );
    assert_eq!(cloud.name(), "cloud");
    assert!(cloud
        .embed(&[])
        .await
        .expect("cloud empty embed")
        .is_empty());

    for (provider, model, dims, expected_name) in [
        (
            "managed",
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
            "cloud",
        ),
        ("voyage", "", 0, "voyage"),
        ("cohere", "", 0, "cohere"),
        ("openai", "text-embedding-3-small", 1536, "openai"),
        ("custom:http://127.0.0.1:9", "custom-embedding", 2, "openai"),
        ("none", "", 0, "none"),
    ] {
        let embedder =
            create_embedding_provider(provider, model, dims).expect("provider should construct");
        assert_eq!(embedder.name(), expected_name);
    }

    match create_embedding_provider("unknown", "m", 1) {
        Ok(_) => panic!("unknown provider should fail"),
        Err(err) => assert!(err.to_string().contains("unknown embedding provider")),
    }

    let default_cloud = openhuman_core::openhuman::embeddings::default_embedding_provider();
    assert_eq!(default_cloud.name(), "cloud");
    let default_local = openhuman_core::openhuman::embeddings::default_local_embedding_provider();
    assert_eq!(default_local.name(), "ollama");

    for (provider, model, dims, key, endpoint, expected_name) in [
        (
            "managed",
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            1024,
            "ignored",
            None,
            "cloud",
        ),
        (
            "voyage",
            "voyage-3-large",
            1024,
            "voyage-key",
            None,
            "voyage",
        ),
        ("ollama", DEFAULT_OLLAMA_MODEL, 1024, "", None, "ollama"),
        (
            "openai",
            "text-embedding-3-small",
            1536,
            "openai-key",
            None,
            "openai",
        ),
        (
            "cohere",
            "embed-english-v3.0",
            1024,
            "cohere-key",
            None,
            "cohere",
        ),
        (
            "custom",
            "custom-model",
            768,
            "custom-key",
            Some("http://127.0.0.1:9"),
            "openai",
        ),
        (
            "custom:http://127.0.0.1:8",
            "custom-model",
            768,
            "custom-key",
            None,
            "openai",
        ),
        ("none", "", 0, "", None, "none"),
    ] {
        let embedder =
            create_embedding_provider_with_credentials(provider, model, dims, key, endpoint)
                .expect("provider with credentials should construct");
        assert_eq!(embedder.name(), expected_name);
    }

    match create_embedding_provider_with_credentials("bogus", "m", 1, "k", None) {
        Ok(_) => panic!("unknown provider with credentials should fail"),
        Err(err) => assert!(err.to_string().contains("unknown embedding provider")),
    }
}

#[tokio::test]
async fn ollama_embed_preserves_positions_and_validates_request_and_response() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(
                body.get("model").and_then(Value::as_str),
                Some("mock-ollama")
            );
            assert_eq!(
                body.pointer("/input/0").and_then(Value::as_str),
                Some("alpha")
            );
            assert_eq!(
                body.pointer("/input/1").and_then(Value::as_str),
                Some("beta")
            );
            Json(json!({ "embeddings": [[1.0, 2.0], [3.0, 4.0]] }))
        }),
    );
    let base_url = serve_mock_ollama(app).await;
    let provider = OllamaEmbedding::try_new(&base_url, "mock-ollama", 2).expect("provider");

    let vectors = provider
        .embed(&[" alpha ", "", "beta", "   "])
        .await
        .expect("ollama embed");
    assert_eq!(
        vectors,
        vec![vec![1.0, 2.0], vec![], vec![3.0, 4.0], vec![]]
    );

    let all_blank = provider.embed(&["", " \n\t "]).await.expect("blank embed");
    assert_eq!(all_blank, vec![Vec::<f32>::new(), Vec::<f32>::new()]);
}

#[tokio::test]
async fn ollama_embed_reports_malformed_count_dimension_and_transport_errors() {
    let count_url = serve_mock_ollama(Router::new().route(
        "/api/embed",
        post(|| async { Json(json!({ "embeddings": [[1.0]] })) }),
    ))
    .await;
    let count_provider = OllamaEmbedding::try_new(&count_url, "m", 1).expect("count provider");
    assert!(count_provider
        .embed(&["a", "b"])
        .await
        .expect_err("count mismatch")
        .to_string()
        .contains("count mismatch"));

    let dim_url = serve_mock_ollama(Router::new().route(
        "/api/embed",
        post(|| async { Json(json!({ "embeddings": [[1.0, 2.0, 3.0]] })) }),
    ))
    .await;
    let dim_provider = OllamaEmbedding::try_new(&dim_url, "m", 2).expect("dim provider");
    assert!(dim_provider
        .embed(&["a"])
        .await
        .expect_err("dimension mismatch")
        .to_string()
        .contains("dimension mismatch"));

    let malformed_url = serve_mock_ollama(Router::new().route(
        "/api/embed",
        post(|| async { (StatusCode::OK, "not json") }),
    ))
    .await;
    let malformed_provider =
        OllamaEmbedding::try_new(&malformed_url, "m", 2).expect("malformed provider");
    assert!(malformed_provider
        .embed(&["a"])
        .await
        .expect_err("malformed response")
        .to_string()
        .contains("parse failed"));

    let refused = OllamaEmbedding::try_new("http://127.0.0.1:1", "m", 2).expect("refused provider");
    assert!(refused
        .embed(&["a"])
        .await
        .expect_err("connection refused")
        .to_string()
        .contains("is Ollama running"));
}

#[tokio::test]
async fn ollama_embed_recovers_nan_batch_with_per_text_fallback() {
    let app = Router::new().route(
        "/api/embed",
        post(|Json(body): Json<Value>| async move {
            let inputs = body
                .get("input")
                .and_then(Value::as_array)
                .expect("input array");
            if inputs.len() > 1 {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    r#"{"error":"failed to encode response: json: unsupported value: NaN"}"#
                        .to_string(),
                );
            }
            if inputs.first().and_then(Value::as_str) == Some("bad") {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unsupported value: nan".to_string(),
                );
            }
            (
                StatusCode::OK,
                json!({ "embeddings": [[9.0, 8.0]] }).to_string(),
            )
        }),
    );
    let base_url = serve_mock_ollama(app).await;
    let provider = OllamaEmbedding::try_new(&base_url, "mock-ollama", 2).expect("provider");

    let vectors = provider
        .embed(&["good", "bad", " "])
        .await
        .expect("nan batch recovery");
    assert_eq!(vectors, vec![vec![9.0, 8.0], vec![], vec![]]);

    let single_nan = provider.embed(&["bad"]).await.expect("single nan recovery");
    assert_eq!(single_nan, vec![Vec::<f32>::new()]);
}
