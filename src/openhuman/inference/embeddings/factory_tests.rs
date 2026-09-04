use super::*;
use crate::openhuman::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use std::collections::HashMap;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config
}

/// #5356 regression (the active production cause). Sign-in stores the
/// `app-session` token under the user-scoped config dir via
/// `AuthService::from_config`; the managed/cloud embedder must derive its
/// credential scope from that SAME `(config.config_path.parent(),
/// config.secrets.encrypt)`. The pre-fix hardcode `(None, true)` resolved to
/// `default_state_dir()` (`~/.openhuman` root) with encryption forced on —
/// the wrong file — so a signed-in user got "No backend session". Setting
/// `encrypt=false` also proves the flag is read from config, not hardcoded.
#[test]
fn managed_credential_scope_mirrors_config_not_default_state_dir() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.secrets.encrypt = false;

    let (dir, encrypt) = managed_credential_scope(&config);
    assert_eq!(
        dir.as_deref(),
        config.config_path.parent(),
        "managed embedder must use the config-scoped credential dir, not default_state_dir()"
    );
    assert!(
        !encrypt,
        "encrypt must reflect config.secrets.encrypt, not the hardcoded `true`"
    );
}

/// End-to-end: a token stored exactly as sign-in stores it (via
/// `AuthService::from_config`) is recovered by the scope the managed branch
/// feeds the cloud embedder's bearer resolver. This is the round-trip the
/// old hardcode broke.
#[test]
fn managed_scope_resolves_signin_stored_app_session_token() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Sign-in writes the app-session token here.
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            "sess-tok-5356",
            HashMap::new(),
            true,
        )
        .unwrap();

    // The exact scope the managed branch uses must recover it.
    let (dir, encrypt) = managed_credential_scope(&config);
    let resolved = AuthService::new(dir.as_deref().unwrap(), encrypt)
        .get_provider_bearer_token(APP_SESSION_PROVIDER, None)
        .unwrap();
    assert_eq!(
        resolved.as_deref(),
        Some("sess-tok-5356"),
        "managed embedder scope must resolve the app-session token sign-in stored"
    );

    // Isolation: a DIFFERENT scope — what the old `(None, true)` hardcode
    // resolved to via `default_state_dir()` (root `~/.openhuman`) instead of
    // the user-scoped config dir — must NOT see the token. This is the half
    // that fails if managed construction ignores `config`.
    let default_like = TempDir::new().unwrap();
    let wrong_scope = AuthService::new(default_like.path(), encrypt)
        .get_provider_bearer_token(APP_SESSION_PROVIDER, None)
        .unwrap();
    assert!(
        wrong_scope.is_none(),
        "app-session token must be isolated to the config scope, not a default/root scope"
    );
}

/// The `managed`/`cloud` arm builds a cloud embedder carrying the requested
/// dimensions (exercises the config-aware factory's managed branch). Mirrors
/// the existing `factory_managed`/`factory_cloud` assertions (name + dims).
#[test]
fn config_aware_factory_builds_managed_provider() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    for provider in ["managed", "cloud"] {
        let p = create_embedding_provider_with_config(
            &config,
            provider,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
            "",
            None,
        )
        .expect("managed/cloud builds via config-aware factory");
        assert_eq!(p.name(), "cloud");
        assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
    }
}

/// #6032: both the config-URL and default-URL paths of the `"ollama"` arm must
/// construct successfully. This is the build-side smoke check; the *binding*
/// proof (that `config.local_ai.base_url` is the URL actually used) is
/// [`config_aware_factory_ollama_embeds_against_config_base_url`] below —
/// `create_embedding_provider_with_config` returns `Box<dyn EmbeddingProvider>`,
/// whose trait surface is `name`/`model_id`/`dimensions`/`embed`, so there is
/// no `base_url()` accessor to assert against here.
#[test]
fn config_aware_factory_ollama_honours_config_base_url() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.local_ai.base_url = Some("http://custom-ollama:12345".to_string());

    // Building must succeed with the custom URL.
    let p =
        create_embedding_provider_with_config(&config, "ollama", "nomic-embed-text", 768, "", None)
            .expect("ollama provider must build with a custom local_ai.base_url");
    assert_eq!(p.name(), "ollama");
    assert_eq!(p.dimensions(), 768);
    // The default-URL build (no custom config) must also succeed and produce
    // the same provider type.
    let mut default_config = test_config(&tmp);
    default_config.local_ai.base_url = None;
    let default_p = create_embedding_provider_with_config(
        &default_config,
        "ollama",
        "nomic-embed-text",
        768,
        "",
        None,
    )
    .expect("ollama provider must build with default URL");
    assert_eq!(default_p.name(), "ollama");
}

/// #6032 binding proof: an `"ollama"` provider built through
/// `create_embedding_provider_with_config` must send its embed request to
/// `config.local_ai.base_url`, not the env-only `ollama_base_url()` default. A
/// local mock stands in for the Ollama daemon at a random port set as
/// `local_ai.base_url`; a regression to the credential-store path (which calls
/// `ollama_base_url()` → `localhost:11434`) would never hit the mock. This is
/// the assertion the build-only test above cannot make, since the returned
/// trait object exposes no URL accessor.
#[tokio::test]
async fn config_aware_factory_ollama_embeds_against_config_base_url() {
    use std::sync::{Arc, Mutex};

    use axum::{extract::State, routing::post, Json, Router};

    // Mock Ollama `/api/embed`: records that it was hit and echoes a
    // 3-dimensional vector (matching the requested `dims`).
    #[derive(Clone, Default)]
    struct Hit {
        count: Arc<Mutex<usize>>,
    }
    let hit = Hit::default();
    let app = Router::new()
        .route(
            "/api/embed",
            post(
                |State(h): State<Hit>, Json(_body): Json<serde_json::Value>| async move {
                    *h.count.lock().unwrap() += 1;
                    Json(serde_json::json!({ "embeddings": [[0.1_f32, 0.2, 0.3]] }))
                },
            ),
        )
        .with_state(hit.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.local_ai.base_url = Some(base.clone());

    let provider =
        create_embedding_provider_with_config(&config, "ollama", "nomic-embed-text", 3, "", None)
            .expect("ollama provider builds with a config base_url");

    let vectors = provider
        .embed(&["binding probe"])
        .await
        .expect("factory-built ollama embedder must reach the config base_url");
    assert_eq!(vectors.first().map(|v| v.len()).unwrap_or(0), 3);
    assert!(
        *hit.count.lock().unwrap() >= 1,
        "embed must hit the configured local_ai.base_url, proving the ollama arm is config-aware"
    );
}

/// Non-managed providers delegate unchanged to the credentialed factory —
/// the config scope has no effect on BYO-key / local providers.
#[test]
fn config_aware_factory_delegates_non_managed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let p =
        create_embedding_provider_with_config(&config, "voyage", "voyage-3-large", 1024, "k", None)
            .expect("voyage builds via delegated path");
    assert_eq!(p.dimensions(), 1024);

    // Unknown provider still surfaces the same error, not a panic.
    assert!(
        create_embedding_provider_with_config(&config, "nope", "m", 1, "", None).is_err(),
        "unknown provider must error through the delegated factory"
    );
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
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            "sess-scoped-5356",
            HashMap::new(),
            true,
        )
        .unwrap();

    // `OpenHumanCloudEmbedding::new` bakes the base URL at construction, so
    // BACKEND_URL only needs to point at the mock while the factory builds —
    // held under the crate-wide backend-env lock (shared with `api::config`'s
    // own BACKEND_URL tests) so the process-global env can't race.
    let provider = {
        let _env_guard = crate::api::config::backend_env_test_lock();
        let prev = std::env::var("BACKEND_URL").ok();
        std::env::set_var("BACKEND_URL", &base);
        let built = create_embedding_provider_with_config(
            &config,
            "managed",
            "voyage-3-large",
            3,
            "",
            None,
        )
        .expect("managed provider builds via config-aware factory");
        match prev {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }
        built
    };

    // Embed: the lazy bearer resolver reads the config scope, finds the
    // token, and authenticates to the mock.
    let vectors = provider
        .embed(&["binding probe"])
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
/// scope "Test connection" uses. The pre-fix keyless
/// [`default_embedding_provider`] resolved `default_state_dir()` with
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
                    let embedding = vec![0.1_f32; DEFAULT_CLOUD_EMBEDDING_DIMENSIONS];
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "object": "embedding", "index": 0, "embedding": embedding }],
                        "model": DEFAULT_CLOUD_EMBEDDING_MODEL,
                    }))
                },
            ),
        )
        .with_state(captured.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The app-session token lives ONLY under the config credential scope.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            "default",
            "sess-default-5501",
            HashMap::new(),
            true,
        )
        .unwrap();

    let provider = {
        let _env_guard = crate::api::config::backend_env_test_lock();
        let prev = std::env::var("BACKEND_URL").ok();
        std::env::set_var("BACKEND_URL", &base);
        let built = default_embedding_provider_with_config(&config);
        match prev {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }
        built
    };

    let vectors = provider
        .embed(&["binding probe"])
        .await
        .expect("config-scoped default embedder must resolve the app-session token and embed");
    assert_eq!(
        vectors.first().map(|v| v.len()).unwrap_or(0),
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS
    );
    assert_eq!(
        captured.auth.lock().unwrap().as_deref(),
        Some("Bearer sess-default-5501"),
        "the memory client's default embedder must authenticate with the config-scoped token"
    );
}
