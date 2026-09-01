//! Factory functions for creating embedding providers.

use std::path::PathBuf;
use std::sync::Arc;

use super::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use super::provider_trait::{EmbeddingProvider, TinyAgentsEmbeddingProvider};
use crate::openhuman::config::Config;
use tinyinference::embeddings::EmbeddingModel;
use tinyinference::embeddings::{
    CohereEmbeddingModel, NoopEmbeddingModel, OllamaEmbeddingModel, OpenAiEmbeddingModel,
    VoyageEmbeddingModel,
};

/// Build the provider for an OpenAI-compatible custom endpoint.
///
/// `dims == 0` is the dimension-agnostic PROBE mode (issue #4056): send no
/// `dimensions` param, accept whatever length comes back, let the caller adopt
/// it. tinyinference's `OpenAiEmbeddingModel::embed` used to implement exactly
/// that (its `parse_vectors` still skips the length guard when
/// `dimensions == 0`), but the tinyagents pointer refresh brought a version
/// whose `embed()` rejects `dimensions == 0` outright before the request is
/// built — the two halves of that file now contradict, and the refusal breaks
/// the Test-connection and save-time probes for every model outside the
/// `text-embedding-3-*` family. Until that is reconciled upstream, the probe
/// mode is served host-side by [`DimensionAgnosticOpenAiProbe`]; delete that
/// type and route `dims == 0` back through `openai_model` once upstream
/// honours it again.
fn custom_openai_provider(
    base_url: &str,
    api_key: &str,
    model: &str,
    dims: usize,
) -> Box<dyn EmbeddingProvider> {
    if dims == 0 {
        Box::new(DimensionAgnosticOpenAiProbe::new(
            openai_model(base_url, api_key, model, 0, false),
            api_key,
        ))
    } else {
        TinyAgentsEmbeddingProvider::boxed(openai_model(base_url, api_key, model, dims, false))
    }
}

/// The dimension-agnostic probe for an OpenAI-compatible endpoint.
///
/// Exists only because upstream's `embed()` refuses `dimensions == 0` (see
/// [`custom_openai_provider`]). One request shape, deliberately minimal: POST
/// `{model, input}` — never a `dimensions` param — bearer auth when a key is
/// present, and no vector-length guard, because learning the endpoint's real
/// length is the entire point of the call.
struct DimensionAgnosticOpenAiProbe {
    /// Used for URL building, naming and the signature — not for `embed`.
    inner: OpenAiEmbeddingModel,
    /// Kept host-side because the inner model does not expose its key.
    api_key: String,
}

impl DimensionAgnosticOpenAiProbe {
    fn new(inner: OpenAiEmbeddingModel, api_key: &str) -> Self {
        Self {
            inner,
            api_key: api_key.to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for DimensionAgnosticOpenAiProbe {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn dimensions(&self) -> usize {
        0
    }

    fn signature(&self) -> String {
        self.inner.signature()
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.inner.embeddings_url();
        let mut request = reqwest::Client::new().post(&url).json(&serde_json::json!({
            "model": self.inner.model(),
            "input": texts,
        }));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = request
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("openai embeddings request to {url} failed: {e}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("openai embeddings body read failed: {e}"))?;
        if !status.is_success() {
            anyhow::bail!("openai embeddings returned HTTP {status}: {text}");
        }
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let data = value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("openai embeddings response missing `data` array"))?;
        if data.len() != texts.len() {
            anyhow::bail!(
                "openai embed count mismatch: sent {} texts, got {} embeddings",
                texts.len(),
                data.len()
            );
        }
        let mut vectors: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for item in data {
            let index = item
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .filter(|i| *i < texts.len())
                .ok_or_else(|| anyhow::anyhow!("openai embedding is missing a valid `index`"))?;
            let embedding = item
                .get("embedding")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    anyhow::anyhow!("openai embeddings response missing `embedding` array")
                })?;
            let vector = embedding
                .iter()
                .map(|n| {
                    n.as_f64().map(|v| v as f32).ok_or_else(|| {
                        anyhow::anyhow!("openai embeddings response contains a non-numeric value")
                    })
                })
                .collect::<anyhow::Result<Vec<f32>>>()?;
            vectors[index] = Some(vector);
        }
        vectors
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                v.ok_or_else(|| anyhow::anyhow!("openai embeddings response is missing index {i}"))
            })
            .collect()
    }
}

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

/// The members of that family, by name, for a consumer that cannot call the
/// predicate.
///
/// The tinymemory module's `EmbeddingHost::model_supports_dimensions` is
/// synchronous and runs inside the module, so it is told a list at load time
/// (`modules::ops::module_config`) rather than asking over the bus. Absent from
/// the list means "does not support it", the safe direction: the engine omits
/// the parameter instead of writing a batch the provider rejects halfway.
pub(crate) const MODELS_SUPPORTING_DIMENSIONS: [&str; 2] =
    ["text-embedding-3-small", "text-embedding-3-large"];

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
                tinyinference::embeddings::VOYAGE_API_BASE,
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
            Ok(custom_openai_provider(base_url, "", model, dims))
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
                tinyinference::embeddings::VOYAGE_API_BASE,
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
            Ok(custom_openai_provider(url, api_key, model, dims))
        }
        name if name.starts_with("custom:") => {
            let url = custom_endpoint.unwrap_or_else(|| name.strip_prefix("custom:").unwrap_or(""));
            Ok(custom_openai_provider(url, api_key, model, dims))
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

/// Returns the default embedding provider — cloud (OpenHuman backend, Voyage) —
/// scoped to `config`'s credential store.
///
/// This is the [`default_embedding_provider`] every caller that holds a
/// `&Config` must use. It threads the caller's real credential-store location
/// ([`managed_credential_scope`]) into the cloud embedder's bearer resolver — the
/// same `(state_dir, encrypt)` pair sign-in wrote the `app-session` token to — so
/// a signed-in user's ingest/seal embeds read the session they actually have.
///
/// The config-less [`default_embedding_provider`] hardcodes `(None, true)` and so
/// resolves `default_state_dir()` with encryption forced on; that only lands on
/// the right store for a default-root, encrypted, single-user install. Routing
/// the memory client's inline embedder through the keyless constructor is what
/// made a signed-in user's ingested documents persist vector-less — "Test
/// connection" passed (config-scoped) while the embed batch silently failed
/// (keyless scope) — #5501.
pub fn default_embedding_provider_with_config(config: &Config) -> Arc<dyn EmbeddingProvider> {
    let (state_dir, encrypt_secrets) = managed_credential_scope(config);
    // Never log `state_dir`: the user-scoped path embeds the OS username and/or
    // `users/<uid>` (PII). Log only the non-identifying flag.
    log::debug!(
        "[embeddings::factory] building default managed embedder from config credential scope (encrypt={encrypt_secrets})"
    );
    Arc::new(OpenHumanCloudEmbedding::new(
        None,
        state_dir,
        encrypt_secrets,
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    ))
}

/// Returns the default embedding provider — cloud (OpenHuman backend, Voyage).
///
/// The cloud embedder lazily resolves the session JWT and API URL on each
/// call, so this can be constructed before login completes; the first
/// `embed()` will fail with a clear message if the user is unauthenticated.
///
/// **Keyless — prefer [`default_embedding_provider_with_config`].** This hardcodes
/// `(None, true)` for the credential scope, resolving `default_state_dir()`
/// (`~/.openhuman` root, or `users/<active>` post-#5427) with encryption forced
/// on. That reads the wrong store whenever the caller's config disables secret
/// encryption or roots the workspace/user elsewhere than the process default
/// (#5356 / #5501). Only callers that genuinely hold no `&Config` should use it.
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
#[path = "factory_tests.rs"]
mod tests;
