use super::*;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::credentials::AuthService;
use std::collections::HashMap;
use tinyinference_embeddings::{DEFAULT_CLOUD_DIMENSIONS, DEFAULT_CLOUD_MODEL};

fn test_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn state_dir_of(config: &Config) -> std::path::PathBuf {
    config
        .config_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default()
}

/// Builds the embedder the way the core's managed factory does: through the
/// registered embedding provider, with the config's credential scope.
fn build_through_registry(
    config: &Config,
    model: &str,
    dimensions: usize,
) -> ManagedEmbeddingModel {
    ManagedEmbeddingModel::from_request(&EmbeddingRequest {
        config: Some(config),
        model,
        dimensions,
        chat: None,
        state_dir: state_dir_of(config),
        secrets_encrypt: config.secrets.encrypt,
    })
}

/// A stored TinyHumans API key authenticates cloud embeddings on its own (no
/// session), sent as `Authorization: Bearer <key>` like managed inference.
#[tokio::test]
async fn api_key_alone_authenticates_cloud_embeddings() {
    use axum::{http::HeaderMap, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Option<String>>> = Arc::default();
    let app = Router::new().route(
        "/openai/v1/embeddings",
        post({
            let seen = Arc::clone(&seen);
            move |headers: HeaderMap| async move {
                *seen.lock().unwrap() = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                Json(serde_json::json!({
                    "object": "list",
                    "data": [{ "object": "embedding", "index": 0, "embedding": [0.5, 0.25] }],
                    "model": "m"
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let state = tempfile::tempdir().unwrap();
    openhuman_embed::__host::security::credentials::api_key::store_api_key_in(
        state.path(),
        false,
        "tiny_test_emb",
    )
    .unwrap();
    let provider = ManagedEmbeddingModel::new(
        Some(format!("http://{addr}")),
        state.path().to_path_buf(),
        false,
        "m",
        2,
        None,
    );

    let vectors = provider
        .embed(&["hello".to_string()])
        .await
        .expect("an API key alone embeds");
    assert_eq!(vectors.len(), 1);
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("Bearer tiny_test_emb")
    );
}

/// The key is never put on a plaintext non-loopback wire.
#[tokio::test]
async fn api_key_is_refused_over_plaintext_non_loopback() {
    let state = tempfile::tempdir().unwrap();
    openhuman_embed::__host::security::credentials::api_key::store_api_key_in(
        state.path(),
        false,
        "tiny_test_emb",
    )
    .unwrap();
    let provider = ManagedEmbeddingModel::new(
        Some("http://embeddings.example.test".into()),
        state.path().to_path_buf(),
        false,
        "m",
        2,
        None,
    );
    let err = provider.embed(&["hello".to_string()]).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("refusing to send the TinyHumans API key"),
        "{err}"
    );
}

/// Drains `rx` until the cloud-embedding disclosure for `marker` arrives.
/// Tolerates unrelated events: the bus is process-wide.
async fn find_embedding_disclosure(
    rx: &mut tinybus::events::EventReceiver<openhuman_embed::__host::core::events::DomainEvent>,
    marker: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    loop {
        match rx.recv().await {
            Some(openhuman_embed::__host::core::events::DomainEvent::ExternalTransferPending {
                descriptor,
                thread_id,
                client_id,
                request_id,
            }) if descriptor.service == marker => return (thread_id, client_id, request_id),
            Some(_) => continue,
            None => panic!("the bus closed before the disclosure arrived"),
        }
    }
}

fn unreachable_embedder(model: &str) -> ManagedEmbeddingModel {
    ManagedEmbeddingModel::new(
        Some("http://127.0.0.1:0".into()),
        std::env::temp_dir().join("openhuman_embeddings_routing_state"),
        false,
        model,
        DEFAULT_CLOUD_DIMENSIONS,
        None,
    )
}

/// A cloud-embedding call made during a web-chat turn discloses its egress to
/// that chat. The chat rides the embedder explicitly (built for the turn from
/// its approval context); it is not read from the running task, which is why
/// this call, awaited on a plain test task, still routes.
#[tokio::test]
async fn a_cloud_embedding_call_in_a_chat_turn_discloses_to_that_chat() {
    openhuman_embed::__host::core::bus::init()
        .await
        .expect("bus init");
    let mut rx = openhuman_embed::__host::core::bus::BUS
        .get()
        .unwrap()
        .receiver();
    let marker = "embed-routed-to-the-turns-chat";
    let chat = ApprovalChatContext {
        thread_id: "thread-embed".into(),
        client_id: "client-embed".into(),
        request_id: Some("req-embed".into()),
    };
    let embedder = unreachable_embedder(marker).routed_to(Some(chat));

    // The transport has nowhere to go; the disclosure precedes it.
    let _ = embedder
        .embed(&["what did I say about invoices?".to_string()])
        .await;

    let (thread_id, client_id, request_id) = find_embedding_disclosure(&mut rx, marker).await;
    assert_eq!(thread_id.as_deref(), Some("thread-embed"));
    assert_eq!(client_id.as_deref(), Some("client-embed"));
    assert_eq!(request_id.as_deref(), Some("req-embed"));
}

/// An embedder built with no chat (background sync, the embedding RPC) still
/// discloses, unrouted, and a blank or empty batch discloses nothing because
/// nothing leaves.
#[tokio::test]
async fn an_unrouted_embedder_discloses_without_a_chat() {
    openhuman_embed::__host::core::bus::init()
        .await
        .expect("bus init");
    let mut rx = openhuman_embed::__host::core::bus::BUS
        .get()
        .unwrap()
        .receiver();
    let marker = "embed-without-a-chat";
    let embedder = unreachable_embedder(marker);

    let _ = embedder.embed(&[]).await;
    let _ = embedder.embed(&["   ".to_string()]).await;
    let _ = embedder.embed(&["real text".to_string()]).await;

    // The first disclosure for this marker is the third call's: the empty and
    // blank batches published nothing.
    let (thread_id, client_id, request_id) = find_embedding_disclosure(&mut rx, marker).await;
    assert_eq!((thread_id, client_id, request_id), (None, None, None));
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            find_embedding_disclosure(&mut rx, marker)
        )
        .await
        .is_err(),
        "only the batch that leaves is disclosed"
    );
}

/// `routed_to` re-routes a shared embedder per turn without touching the
/// original: each turn's disclosure reaches its own chat.
#[tokio::test]
async fn routing_is_per_embedder_not_shared() {
    openhuman_embed::__host::core::bus::init()
        .await
        .expect("bus init");
    let mut rx = openhuman_embed::__host::core::bus::BUS
        .get()
        .unwrap()
        .receiver();
    let marker = "embed-routed-per-turn";
    let base = unreachable_embedder(marker);
    let chat = |thread: &str| ApprovalChatContext {
        thread_id: thread.into(),
        client_id: "c".into(),
        request_id: None,
    };

    let _ = base
        .routed_to(Some(chat("thread-a")))
        .embed(&["a".into()])
        .await;
    assert_eq!(
        find_embedding_disclosure(&mut rx, marker)
            .await
            .0
            .as_deref(),
        Some("thread-a")
    );
    let _ = base
        .routed_to(Some(chat("thread-b")))
        .embed(&["b".into()])
        .await;
    assert_eq!(
        find_embedding_disclosure(&mut rx, marker)
            .await
            .0
            .as_deref(),
        Some("thread-b")
    );
    let _ = base.embed(&["c".into()]).await;
    assert_eq!(find_embedding_disclosure(&mut rx, marker).await.0, None);
}

/// End-to-end binding proof (#5356): a provider built **through
/// `create_embedding_provider_with_config`** must authenticate with the
/// `app-session` token stored under the *config* credential scope. A local
/// mock stands in for the cloud backend (no external network) and captures
/// the bearer it receives. A regression to `OpenHumanCloudEmbedding::new(None,
/// None, true, …)` would resolve the token from `default_state_dir()` — a
/// different directory with no token — and fail with "No backend session"
/// before any request, so this test fails if the factory ignores `config`.
#[tokio::test]
async fn factory_managed_provider_authenticates_with_config_scoped_token() {
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};

    // Mock cloud embeddings backend at {BACKEND_URL}/openai/v1/embeddings.
    #[derive(Clone, Default)]
    struct Captured {
        auth: Arc<Mutex<Option<String>>>,
    }
    let captured = Captured::default();
    let app = Router::new()
        .route(
            "/openai/v1/embeddings",
            post(
                |State(cap): State<Captured>,
                 headers: HeaderMap,
                 Json(_body): Json<serde_json::Value>| async move {
                    *cap.auth.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "object": "embedding", "index": 0, "embedding": [0.1_f32, 0.2, 0.3] }],
                        "model": "voyage-3-large",
                    }))
                },
            ),
        )
        .with_state(captured.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The app-session token lives ONLY under the config credential scope.
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    AuthService::new(&state_dir_of(&config), config.secrets.encrypt)
        .store_provider_token(
            openhuman_embed::__host::security::credentials::APP_SESSION_PROVIDER,
            "default",
            "sess-scoped-5356",
            HashMap::new(),
            true,
        )
        .unwrap();

    // The managed base URL comes from the backend transport, which (the
    // `cfg(test)` plain one here) honours `config.api_url`: point it at the mock.
    let config = Config {
        api_url: Some(base.clone()),
        ..config
    };
    let provider = build_through_registry(&config, "voyage-3-large", 3);

    // Embed: the lazy bearer resolver reads the config scope, finds the
    // token, and authenticates to the mock.
    let vectors = provider
        .embed(&["binding probe".to_string()])
        .await
        .expect("factory-built managed provider must resolve the config-scoped token and embed");
    assert_eq!(vectors.first().map(|v| v.len()).unwrap_or(0), 3);
    assert_eq!(
        captured.auth.lock().unwrap().as_deref(),
        Some("Bearer sess-scoped-5356"),
        "managed provider must authenticate with the token from the config scope"
    );
}

/// #5501 regression: the memory client's inline embedder is built through
/// [`default_embedding_provider_with_config`], which MUST authenticate with
/// the `app-session` token under the *config* credential scope — the same
/// scope "Test connection" uses. The removed pre-fix keyless constructor
/// resolved `default_state_dir()` with
/// encryption forced on, so a signed-in user's ingested documents embedded
/// with "No backend session" and persisted vector-less while the connection
/// test still passed. A local mock stands in for the cloud backend (no
/// network) and captures the bearer it receives; a regression back to the
/// keyless constructor resolves a scope with no token and never reaches it.
#[tokio::test]
async fn default_provider_with_config_authenticates_with_config_scoped_token() {
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};

    #[derive(Clone, Default)]
    struct Captured {
        auth: Arc<Mutex<Option<String>>>,
    }
    let captured = Captured::default();
    let app = Router::new()
        .route(
            "/openai/v1/embeddings",
            post(
                |State(cap): State<Captured>,
                 headers: HeaderMap,
                 Json(_body): Json<serde_json::Value>| async move {
                    *cap.auth.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    // The embedder validates returned dims against the
                    // requested default, so the mock must echo that width.
                    let embedding = vec![0.1_f32; DEFAULT_CLOUD_DIMENSIONS];
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "object": "embedding", "index": 0, "embedding": embedding }],
                        "model": DEFAULT_CLOUD_MODEL,
                    }))
                },
            ),
        )
        .with_state(captured.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The app-session token lives ONLY under the config credential scope.
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    AuthService::new(&state_dir_of(&config), config.secrets.encrypt)
        .store_provider_token(
            openhuman_embed::__host::security::credentials::APP_SESSION_PROVIDER,
            "default",
            "sess-default-5501",
            HashMap::new(),
            true,
        )
        .unwrap();

    let config = Config {
        api_url: Some(base.clone()),
        ..config
    };
    let provider = build_through_registry(&config, DEFAULT_CLOUD_MODEL, DEFAULT_CLOUD_DIMENSIONS);

    let vectors = provider
        .embed(&["binding probe".to_string()])
        .await
        .expect("config-scoped default embedder must resolve the app-session token and embed");
    assert_eq!(
        vectors.first().map(|v| v.len()).unwrap_or(0),
        DEFAULT_CLOUD_DIMENSIONS
    );
    assert_eq!(
        captured.auth.lock().unwrap().as_deref(),
        Some("Bearer sess-default-5501"),
        "the memory client's default embedder must authenticate with the config-scoped token"
    );
}

/// With no stored `app-session` token and no API key, the bearer resolver
/// short-circuits to the backend-session error before any network call.
#[tokio::test]
async fn embedding_without_a_session_reports_no_backend_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let provider = build_through_registry(&config, "voyage-3-large", 3);
    let err = provider
        .embed(&["hello".to_string()])
        .await
        .expect_err("no session must not embed");
    assert!(err.to_string().contains("No backend session"), "{err}");
}
