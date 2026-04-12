//! Embedding providers for the OpenHuman memory system.
//!
//! This module provides a unified interface for converting text into vector
//! embeddings. It supports multiple providers:
//! - **Fastembed**: Local, high-performance embeddings using ONNX runtime.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API or compatible endpoints.
//! - **Noop**: A fallback provider for keyword-only search.

use async_trait::async_trait;
use parking_lot::Mutex;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

/// Default model name for Fastembed.
pub const DEFAULT_FASTEMBED_MODEL: &str = "BGESmallENV15";
/// Default dimensions for the BGESmallENV15 model.
pub const DEFAULT_FASTEMBED_DIMENSIONS: usize = 384;
/// Interface for embedding providers that convert text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "fastembed", "openai").
    fn name(&self) -> &str;

    /// Returns the number of dimensions in the generated embeddings.
    fn dimensions(&self) -> usize;

    /// Generates embeddings for a batch of strings.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Generates an embedding for a single string.
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }
}

// ── Noop provider (keyword-only fallback) ────────────────────

/// A "no-op" embedding provider used when semantic search is disabled.
/// Returns empty vectors.
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    async fn embed(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }
}

/// Represents the initialization state of the local Fastembed model.
enum FastembedState {
    /// Initial state before the model is loaded.
    Uninitialized,
    /// Model is loaded into memory and ready for inference.
    Ready(fastembed::TextEmbedding),
    /// An error occurred during model loading.
    Failed(String),
}

/// Local embedding provider using the `fastembed-rs` library.
/// Executes in a dedicated blocking thread to avoid stalling the async runtime.
pub struct FastembedEmbedding {
    model: String,
    dims: usize,
    state: Arc<Mutex<FastembedState>>,
}

impl FastembedEmbedding {
    /// Creates a new Fastembed provider with the specified model and dimensions.
    pub fn new(model: &str, dims: usize) -> Self {
        Self {
            model: if model.trim().is_empty() {
                DEFAULT_FASTEMBED_MODEL.to_string()
            } else {
                model.trim().to_string()
            },
            dims: if dims == 0 {
                DEFAULT_FASTEMBED_DIMENSIONS
            } else {
                dims
            },
            state: Arc::new(Mutex::new(FastembedState::Uninitialized)),
        }
    }

    /// Maps a string model name to a `fastembed::EmbeddingModel` enum.
    fn resolve_model(&self) -> fastembed::EmbeddingModel {
        fastembed::EmbeddingModel::from_str(&self.model)
            .unwrap_or(fastembed::EmbeddingModel::BGESmallENV15)
    }

    /// Internal helper to initialize the model on first use.
    fn init_model(&self) -> anyhow::Result<fastembed::TextEmbedding> {
        ensure_fastembed_ort_dylib_path();
        fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(self.resolve_model()).with_show_download_progress(false),
        )
        .map_err(|e| anyhow::anyhow!("fastembed init failed for {}: {e}", self.model))
    }
}

/// Configures the search path for the ONNX Runtime dynamic library.
///
/// This is critical for Fastembed to function across different platforms and
/// installation methods (e.g., local dev, bundled app). It checks several
/// locations in order of priority:
/// 1. `ORT_DYLIB_PATH` environment variable.
/// 2. `ORT_LIB_LOCATION` environment variable.
/// 3. OpenHuman-specific cache directories.
/// 4. Standard system library paths (Linux only).
fn ensure_fastembed_ort_dylib_path() {
    if env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }

    // Check for explicit library location override.
    if let Some(lib_path) = env::var_os("ORT_LIB_LOCATION") {
        let candidate = PathBuf::from(lib_path);
        if candidate.is_file() {
            env::set_var("ORT_DYLIB_PATH", candidate);
            return;
        }

        #[cfg(target_os = "windows")]
        let runtime_lib = candidate.join("onnxruntime.dll");
        #[cfg(target_os = "macos")]
        let runtime_lib = candidate.join("libonnxruntime.dylib");
        #[cfg(target_os = "linux")]
        let runtime_lib = candidate.join("libonnxruntime.so");

        if runtime_lib.exists() {
            env::set_var("ORT_DYLIB_PATH", runtime_lib);
        }
    }

    // Fallback to system-wide paths on Linux.
    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/lib/x86_64-linux-gnu/libonnxruntime.so",
            "/usr/local/lib/libonnxruntime.so",
            "/usr/lib/libonnxruntime.so",
        ] {
            let candidate = PathBuf::from(candidate);
            if candidate.exists() {
                env::set_var("ORT_DYLIB_PATH", candidate);
                return;
            }
        }
    }
}

#[async_trait]
impl EmbeddingProvider for FastembedEmbedding {
    fn name(&self) -> &str {
        "fastembed"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Performs embedding using a blocking task to prevent executor starvation.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let items = texts
            .iter()
            .map(|text| (*text).to_string())
            .collect::<Vec<_>>();
        let state = Arc::clone(&self.state);
        let provider = self.model.clone();

        let join_result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<f32>>> {
            ensure_fastembed_ort_dylib_path();
            let mut guard = state.lock();

            // Lazy initialization of the model on the first request.
            //
            // `fastembed::TextEmbedding::try_new` reaches into the `ort`
            // crate's global environment, which uses a `std::sync::Mutex`.
            // If any previous caller panicked while that mutex was held
            // (common when the ONNX Runtime dylib path is wrong or a
            // background init failed), every subsequent call panics with
            // `"Mutex poisoned"`. Without `catch_unwind`, that panic
            // propagates out of this `spawn_blocking` closure, kills the
            // tokio blocking worker, and surfaces as a process-level
            // stack trace — even though the caller only wanted an error.
            //
            // We trap the panic here, flip our own state to `Failed`, and
            // return a regular `anyhow::Error` so every later call short-
            // circuits on the cached failure without touching `ort` again.
            if matches!(*guard, FastembedState::Uninitialized) {
                let provider_for_init = provider.clone();
                let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fastembed::TextEmbedding::try_new(
                        fastembed::InitOptions::new(
                            fastembed::EmbeddingModel::from_str(&provider_for_init)
                                .unwrap_or(fastembed::EmbeddingModel::BGESmallENV15),
                        )
                        .with_show_download_progress(false),
                    )
                }));

                match init_result {
                    Ok(Ok(model)) => *guard = FastembedState::Ready(model),
                    Ok(Err(err)) => {
                        let message = format!("fastembed init failed for {provider}: {err}");
                        tracing::error!(target: "memory.embeddings", "[embeddings] {message}");
                        *guard = FastembedState::Failed(message);
                    }
                    Err(panic_payload) => {
                        let panic_msg = extract_panic_message(&panic_payload);
                        let message = format!(
                            "fastembed init panicked for {provider}: {panic_msg} — \
                             the ONNX Runtime global environment is in a poisoned state. \
                             Check ORT_DYLIB_PATH / ORT_LIB_LOCATION and restart the \
                             process to retry."
                        );
                        tracing::error!(target: "memory.embeddings", "[embeddings] {message}");
                        *guard = FastembedState::Failed(message);
                    }
                }
            }

            match &mut *guard {
                FastembedState::Ready(model) => {
                    // Also guard the actual embed call — fastembed / ort
                    // can panic on certain inputs or runtime errors, and
                    // we want to surface those as regular errors too.
                    let embed_result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            model.embed(items, None)
                        }));
                    match embed_result {
                        Ok(Ok(vectors)) => Ok(vectors),
                        Ok(Err(e)) => Err(anyhow::anyhow!("fastembed embed failed: {e}")),
                        Err(panic_payload) => {
                            let panic_msg = extract_panic_message(&panic_payload);
                            Err(anyhow::anyhow!("fastembed embed panicked: {panic_msg}"))
                        }
                    }
                }
                FastembedState::Failed(message) => Err(anyhow::anyhow!(message.clone())),
                FastembedState::Uninitialized => {
                    Err(anyhow::anyhow!("fastembed provider did not initialize"))
                }
            }
        })
        .await;

        join_result.map_err(|e| anyhow::anyhow!("fastembed task join failed: {e}"))?
    }
}

/// Best-effort extraction of a readable message from a `catch_unwind` payload.
/// Panics produced by `panic!("...")` downcast to `&'static str` or `String`;
/// everything else falls back to a generic label.
fn extract_panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

// ── OpenAI-compatible embedding provider ─────────────────────

/// Embedding provider for OpenAI and compatible APIs (e.g., LocalAI, Ollama).
pub struct OpenAiEmbedding {
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
}

impl OpenAiEmbedding {
    /// Creates a new OpenAI-style provider.
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dims,
        }
    }

    /// Internal helper to build an HTTP client with proxy support.
    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("memory.embeddings")
    }

    /// Checks if the base URL includes a specific path (e.g., /api/v1).
    fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    /// Checks if the URL already ends with /embeddings.
    fn has_embeddings_endpoint(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        url.path().trim_end_matches('/').ends_with("/embeddings")
    }

    /// Constructs the final URL for the embeddings endpoint.
    fn embeddings_url(&self) -> String {
        if self.has_embeddings_endpoint() {
            return self.base_url.clone();
        }

        if self.has_explicit_api_path() {
            format!("{}/embeddings", self.base_url)
        } else {
            format!("{}/v1/embeddings", self.base_url)
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedding {
    fn name(&self) -> &str {
        "openai"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Sends a POST request to the embedding API.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self
            .http_client()
            .post(self.embeddings_url())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Embedding API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding response: missing 'data'"))?;

        let mut embeddings = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| anyhow::anyhow!("Invalid embedding item"))?;

            #[allow(clippy::cast_possible_truncation)]
            let vec: Vec<f32> = embedding
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect();

            embeddings.push(vec);
        }

        Ok(embeddings)
    }
}

// ── Factory ──────────────────────────────────────────────────

/// Creates an embedding provider based on the specified name and configuration.
///
/// Supports "fastembed", "openai", and "custom:<url>".
pub fn create_embedding_provider(
    provider: &str,
    api_key: Option<&str>,
    model: &str,
    dims: usize,
) -> Box<dyn EmbeddingProvider> {
    match provider {
        "fastembed" => Box::new(FastembedEmbedding::new(model, dims)),
        "openai" => {
            let key = api_key.unwrap_or("");
            Box::new(OpenAiEmbedding::new(
                "https://api.openai.com",
                key,
                model,
                dims,
            ))
        }
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            let key = api_key.unwrap_or("");
            Box::new(OpenAiEmbedding::new(base_url, key, model, dims))
        }
        _ => Box::new(NoopEmbedding),
    }
}

/// Returns the default local embedding provider (Fastembed).
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(FastembedEmbedding::new(
        DEFAULT_FASTEMBED_MODEL,
        DEFAULT_FASTEMBED_DIMENSIONS,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_name() {
        let p = NoopEmbedding;
        assert_eq!(p.name(), "none");
        assert_eq!(p.dimensions(), 0);
    }

    #[tokio::test]
    async fn noop_embed_returns_empty() {
        let p = NoopEmbedding;
        let result = p.embed(&["hello"]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn factory_none() {
        let p = create_embedding_provider("none", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn factory_openai() {
        let p = create_embedding_provider("openai", Some("key"), "text-embedding-3-small", 1536);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_fastembed() {
        let p = create_embedding_provider("fastembed", None, DEFAULT_FASTEMBED_MODEL, 384);
        assert_eq!(p.name(), "fastembed");
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn factory_custom_url() {
        let p = create_embedding_provider("custom:http://localhost:1234", None, "model", 768);
        assert_eq!(p.name(), "openai"); // uses OpenAiEmbedding internally
        assert_eq!(p.dimensions(), 768);
    }

    // ── Edge cases ───────────────────────────────────────────────

    #[tokio::test]
    async fn noop_embed_one_returns_error() {
        let p = NoopEmbedding;
        // embed returns empty vec → pop() returns None → error
        let result = p.embed_one("hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn noop_embed_empty_batch() {
        let p = NoopEmbedding;
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn noop_embed_multiple_texts() {
        let p = NoopEmbedding;
        let result = p.embed(&["a", "b", "c"]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn factory_empty_string_returns_noop() {
        let p = create_embedding_provider("", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn factory_unknown_provider_returns_noop() {
        let p = create_embedding_provider("cohere", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn default_local_provider_uses_fastembed_defaults() {
        let p = default_local_embedding_provider();
        assert_eq!(p.name(), "fastembed");
        assert_eq!(p.dimensions(), DEFAULT_FASTEMBED_DIMENSIONS);
    }

    #[test]
    fn factory_custom_empty_url() {
        // "custom:" with no URL — should still construct without panic
        let p = create_embedding_provider("custom:", None, "model", 768);
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn factory_openai_no_api_key() {
        let p = create_embedding_provider("openai", None, "text-embedding-3-small", 1536);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn openai_trailing_slash_stripped() {
        let p = OpenAiEmbedding::new("https://api.openai.com/", "key", "model", 1536);
        assert_eq!(p.base_url, "https://api.openai.com");
    }

    #[test]
    fn openai_dimensions_custom() {
        let p = OpenAiEmbedding::new("http://localhost", "k", "m", 384);
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn embeddings_url_standard_openai() {
        let p = OpenAiEmbedding::new("https://api.openai.com", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embeddings_url_base_with_v1_no_duplicate() {
        let p = OpenAiEmbedding::new("https://api.example.com/v1", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
    }

    #[test]
    fn embeddings_url_non_v1_api_path_uses_raw_suffix() {
        let p = OpenAiEmbedding::new(
            "https://api.example.com/api/coding/v3",
            "key",
            "model",
            1536,
        );
        assert_eq!(
            p.embeddings_url(),
            "https://api.example.com/api/coding/v3/embeddings"
        );
    }

    #[test]
    fn embeddings_url_custom_full_endpoint() {
        let p = OpenAiEmbedding::new(
            "https://my-api.example.com/api/v2/embeddings",
            "key",
            "model",
            1536,
        );
        assert_eq!(
            p.embeddings_url(),
            "https://my-api.example.com/api/v2/embeddings"
        );
    }
}
