//! Factory functions for creating embedding providers.

use std::path::PathBuf;
use std::sync::Arc;

use super::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use super::provider_trait::{EmbeddingProvider, TinyAgentsEmbeddingProvider};
use crate::openhuman::config::Config;
use tinyagents::harness::embeddings::{
    CohereEmbeddingModel, NoopEmbeddingModel, OllamaEmbeddingModel, OpenAiEmbeddingModel,
    VoyageEmbeddingModel,
};

fn openai_model(
    base_url: &str,
    api_key: &str,
    model: &str,
    dims: usize,
    required_key: bool,
) -> OpenAiEmbeddingModel {
    OpenAiEmbeddingModel::new(api_key)
        .with_base_url(base_url)
        .with_model(model)
        .with_dimensions(dims)
        .with_send_dimensions(model_supports_dimensions(model))
        .with_required_api_key(required_key)
}

/// Whether to send the OpenAI `dimensions` request-body parameter for this
/// model. Only the `text-embedding-3-*` family honors it (it's how 3-large is
/// pinned to 1024 = `EMBEDDING_DIM`). Sending it to other models or to
/// arbitrary OpenAI-compatible servers (vLLM, text-embeddings-inference,
/// stricter LocalAI builds) makes those servers 400 on an unknown field, so we
/// gate on the model id rather than the provider kind. (Reviewer sanil-23, #3076.)
pub(crate) fn model_supports_dimensions(model: &str) -> bool {
    model.starts_with("text-embedding-3-")
}

/// Creates an embedding provider based on the specified name and configuration.
///
/// Supported provider names:
/// - `"managed"` / `"cloud"` → OpenHuman backend (Voyage-backed) — default
/// - `"voyage"` → direct Voyage AI API (user's own key)
/// - `"openai"` → OpenAI API (user's own key)
/// - `"cohere"` → Cohere API (user's own key)
/// - `"ollama"` → local Ollama server (opt-in for offline-only installs)
/// - `"custom:<url>"` → OpenAI-compatible endpoint
/// - `"none"` → no-op (keyword-only search, no embeddings)
///
/// Returns an error for unrecognised provider names so configuration
/// mistakes surface immediately rather than silently degrading to
/// keyword-only search.
pub fn create_embedding_provider(
    provider: &str,
    model: &str,
    dims: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "cloud" | "managed" => Ok(Box::new(OpenHumanCloudEmbedding::new(
            None, None, true, model, dims,
        ))),
        "voyage" => Ok(TinyAgentsEmbeddingProvider::boxed(
            VoyageEmbeddingModel::with_options(
                "",
                model,
                dims,
                tinyagents::harness::embeddings::VOYAGE_API_BASE,
            ),
        )),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(TinyAgentsEmbeddingProvider::boxed(
                OllamaEmbeddingModel::try_new(&base_url, model, dims)?,
            ))
        }
        "openai" => Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
            "https://api.openai.com",
            "",
            model,
            dims,
            true,
        ))),
        "cohere" => Ok(TinyAgentsEmbeddingProvider::boxed(
            CohereEmbeddingModel::new("")
                .with_model(model)
                .with_dimensions(dims),
        )),
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                base_url, "", model, dims, false,
            )))
        }
        "none" => Ok(TinyAgentsEmbeddingProvider::boxed(NoopEmbeddingModel)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"managed\", \"voyage\", \"openai\", \"cohere\", \
             \"ollama\", \"custom:<url>\", \"none\""
        )),
    }
}

/// Creates an embedding provider with explicit API key and endpoint.
///
/// Used by the RPC layer when credentials are loaded from the credential
/// store.
pub fn create_embedding_provider_with_credentials(
    provider: &str,
    model: &str,
    dims: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "cloud" | "managed" => Ok(Box::new(OpenHumanCloudEmbedding::new(
            None, None, true, model, dims,
        ))),
        "voyage" => Ok(TinyAgentsEmbeddingProvider::boxed(
            VoyageEmbeddingModel::with_options(
                api_key,
                model,
                dims,
                tinyagents::harness::embeddings::VOYAGE_API_BASE,
            ),
        )),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(TinyAgentsEmbeddingProvider::boxed(
                OllamaEmbeddingModel::try_new(&base_url, model, dims)?,
            ))
        }
        "openai" => Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
            "https://api.openai.com",
            api_key,
            model,
            dims,
            true,
        ))),
        "cohere" => Ok(TinyAgentsEmbeddingProvider::boxed(
            CohereEmbeddingModel::new(api_key)
                .with_model(model)
                .with_dimensions(dims),
        )),
        "custom" => {
            let url = custom_endpoint.unwrap_or("");
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                url, api_key, model, dims, false,
            )))
        }
        name if name.starts_with("custom:") => {
            let url = custom_endpoint.unwrap_or_else(|| name.strip_prefix("custom:").unwrap_or(""));
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                url, api_key, model, dims, false,
            )))
        }
        "none" => Ok(TinyAgentsEmbeddingProvider::boxed(NoopEmbeddingModel)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"managed\", \"voyage\", \"openai\", \"cohere\", \
             \"ollama\", \"custom\", \"none\""
        )),
    }
}

/// Config-aware variant of [`create_embedding_provider_with_credentials`].
///
/// Behaves identically for every provider **except** `managed`/`cloud`. For
/// those it threads the caller's real credential-store location
/// ([`managed_credential_scope`]) into the cloud embedder's bearer resolver — the
/// same `(state_dir, encrypt)` pair
/// [`AuthService::from_config`](crate::openhuman::security::credentials::AuthService::from_config)
/// uses to **store** the `app-session` token at sign-in.
///
/// The keyless constructors hardcode `(None, true)`, which resolves to
/// `default_state_dir()` (`~/.openhuman` root) with encryption forced on. On a
/// shipped desktop `OPENHUMAN_WORKSPACE` is unset and the session token lives
/// under the user-scoped `~/.openhuman/users/<uid>/auth-profiles.json`, so that
/// hardcode reads the *wrong* file and a signed-in user's managed "Test
/// connection" / embed falsely reports "No backend session" (#5356). Callers
/// that hold a `&Config` must route managed construction through here.
pub fn create_embedding_provider_with_config(
    config: &Config,
    provider: &str,
    model: &str,
    dims: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "cloud" | "managed" => {
            let (state_dir, encrypt_secrets) = managed_credential_scope(config);
            // Never log `state_dir`: the user-scoped path embeds the OS username
            // and/or `users/<uid>` (PII). Log only the non-identifying flag.
            log::debug!(
                "[embeddings::factory] building managed embedder from config credential scope (encrypt={encrypt_secrets})"
            );
            Ok(Box::new(OpenHumanCloudEmbedding::new(
                None,
                state_dir,
                encrypt_secrets,
                model,
                dims,
            )))
        }
        // Every other provider is credential-store-agnostic (BYO key or local
        // endpoint), so the existing construction is correct unchanged.
        other => {
            create_embedding_provider_with_credentials(other, model, dims, api_key, custom_endpoint)
        }
    }
}

/// The `(state_dir, encrypt)` the managed/cloud embedder must use to find the
/// `app-session` token. Delegates to
/// [`state_dir_from_config`](crate::openhuman::security::credentials::state_dir_from_config)
/// — the exact helper [`AuthService::from_config`] uses — so the embedder reads
/// the token from the **same** store sign-in wrote it to, including the
/// `"."`-fallback when `config_path` has no parent (a bare filename). Returning
/// the raw parent instead would yield `None` there and silently fall back to
/// `default_state_dir()` — the very divergence this fix removes. Extracted as a
/// pure fn so the #5356 invariant is unit-testable without a network round-trip.
fn managed_credential_scope(config: &Config) -> (Option<PathBuf>, bool) {
    use crate::openhuman::security::credentials::state_dir_from_config;
    (Some(state_dir_from_config(config)), config.secrets.encrypt)
}

/// Returns the default embedding provider — cloud (OpenHuman backend, Voyage).
///
/// The cloud embedder lazily resolves the session JWT and API URL on each
/// call, so this can be constructed before login completes; the first
/// `embed()` will fail with a clear message if the user is unauthenticated.
pub fn default_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(OpenHumanCloudEmbedding::new(
        None,
        None,
        true,
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    ))
}

/// Returns the local Ollama-backed embedding provider. Only used when the
/// caller has explicitly opted into local-only embeddings.
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(TinyAgentsEmbeddingProvider::new(
        OllamaEmbeddingModel::default(),
    ))
}

#[cfg(test)]
mod tests {
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

    /// Non-managed providers delegate unchanged to the credentialed factory —
    /// the config scope has no effect on BYO-key / local providers.
    #[test]
    fn config_aware_factory_delegates_non_managed() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let p = create_embedding_provider_with_config(
            &config,
            "voyage",
            "voyage-3-large",
            1024,
            "k",
            None,
        )
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
        let vectors = provider.embed(&["binding probe"]).await.expect(
            "factory-built managed provider must resolve the config-scoped token and embed",
        );
        assert_eq!(vectors.first().map(|v| v.len()).unwrap_or(0), 3);
        assert_eq!(
            captured.auth.lock().unwrap().as_deref(),
            Some("Bearer sess-scoped-5356"),
            "managed provider must authenticate with the token from the config scope"
        );
    }
}
