//! Ollama-based embedding provider.
//!
//! Calls the local Ollama server's `/api/embed` endpoint for embeddings.
//! This is the preferred local provider: Ollama handles model management,
//! quantization, and GPU acceleration (Metal on macOS, CUDA on Linux/Windows).
//!
//! Default model: `bge-m3` (1024 dimensions). Aligned with the memory
//! tree's fixed on-disk format (`EMBEDDING_DIM=1024`) and the cloud
//! Voyage default (`embedding-v1`, 1024 dims) so embeddings produced by
//! either path are interchangeable.

use async_trait::async_trait;

use super::EmbeddingProvider;

/// Default Ollama base URL.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Default embedding model for Ollama. 1024-dim to match the memory
/// tree's fixed on-disk format and the cloud Voyage default.
pub const DEFAULT_OLLAMA_MODEL: &str = "bge-m3";

/// Default dimensions for `bge-m3`.
pub const DEFAULT_OLLAMA_DIMENSIONS: usize = 1024;

/// Embedding provider backed by a local Ollama instance.
///
/// Ollama must be running and have the configured model pulled.
/// On first embed call, if the model isn't available, Ollama will
/// auto-pull it (this may take a moment on first use).
#[derive(Debug)]
pub struct OllamaEmbedding {
    base_url: String,
    model: String,
    dims: usize,
}

impl OllamaEmbedding {
    /// Creates a new Ollama embedding provider.
    ///
    /// - `base_url`: Ollama server URL (default: `http://localhost:11434`)
    /// - `model`: Model name (default: `bge-m3`)
    /// - `dims`: Expected embedding dimensions (default: 1024)
    pub fn try_new(base_url: &str, model: &str, dims: usize) -> anyhow::Result<Self> {
        let base_url = Self::normalize_base_url(base_url)?;
        let model = Self::normalize_model(model)?;
        let dims = if dims == 0 {
            DEFAULT_OLLAMA_DIMENSIONS
        } else {
            dims
        };

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] OllamaEmbedding created: url={base_url}, model={model}, dims={dims}"
        );

        Ok(Self {
            base_url,
            model,
            dims,
        })
    }

    /// Creates a new Ollama embedding provider, panicking if the configuration
    /// is invalid. Prefer [`Self::try_new`] at runtime boundaries.
    pub fn new(base_url: &str, model: &str, dims: usize) -> Self {
        Self::try_new(base_url, model, dims).expect("invalid Ollama embedding configuration")
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Build an HTTP client with proxy support.
    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("embeddings.ollama")
    }

    fn normalize_base_url(base_url: &str) -> anyhow::Result<String> {
        let raw = if base_url.trim().is_empty() {
            DEFAULT_OLLAMA_URL
        } else {
            base_url.trim()
        };

        let url = reqwest::Url::parse(raw)
            .map_err(|e| anyhow::anyhow!("invalid Ollama base_url `{raw}`: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            anyhow::bail!("invalid Ollama base_url `{raw}`: expected an http:// or https:// URL");
        }
        if !url.username().is_empty() || url.password().is_some() {
            anyhow::bail!(
                "invalid Ollama base_url `{raw}`: configure the server root without credentials"
            );
        }
        if url.query().is_some() || url.fragment().is_some() {
            anyhow::bail!(
                "invalid Ollama base_url `{raw}`: query strings and fragments are not supported"
            );
        }

        let segments: Vec<String> = url
            .path_segments()
            .map(|parts| {
                parts
                    .filter(|part| !part.is_empty())
                    .map(|part| part.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let has_api_suffix = segments.iter().any(|part| part == "api" || part == "v1");
        let is_chat_completions_endpoint = segments.len() >= 2
            && segments[segments.len() - 2] == "chat"
            && segments[segments.len() - 1] == "completions";
        if has_api_suffix || is_chat_completions_endpoint {
            anyhow::bail!(
                "invalid Ollama base_url `{raw}`: configure the Ollama server root \
                 (for example {DEFAULT_OLLAMA_URL}), not an API endpoint such as \
                 /api, /v1, or /chat/completions"
            );
        }

        Ok(url.as_str().trim_end_matches('/').to_string())
    }

    fn normalize_model(model: &str) -> anyhow::Result<String> {
        let model = if model.trim().is_empty() {
            DEFAULT_OLLAMA_MODEL.to_string()
        } else {
            model.trim().to_string()
        };
        if model.to_ascii_lowercase().starts_with("local-") {
            anyhow::bail!(
                "invalid Ollama embedding model `{model}`: `local-*` model IDs are virtual \
                 routing aliases. Configure a real Ollama embedding model such as \
                 `{DEFAULT_OLLAMA_MODEL}`."
            );
        }
        Ok(model)
    }

    /// The embed endpoint URL.
    fn embed_url(&self) -> anyhow::Result<String> {
        let _ = reqwest::Url::parse(&self.base_url)
            .map_err(|e| anyhow::anyhow!("invalid Ollama base_url `{}`: {e}", self.base_url))?;
        Ok(format!("{}/api/embed", self.base_url))
    }
}

impl Default for OllamaEmbedding {
    fn default() -> Self {
        Self::try_new(
            DEFAULT_OLLAMA_URL,
            DEFAULT_OLLAMA_MODEL,
            DEFAULT_OLLAMA_DIMENSIONS,
        )
        .expect("default Ollama embedding configuration must be valid")
    }
}

/// Ollama `/api/embed` request body.
#[derive(serde::Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

/// Ollama `/api/embed` response body.
#[derive(serde::Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    fn name(&self) -> &str {
        "ollama"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Sends texts to Ollama's embed API.
    ///
    /// Blank/whitespace-only entries are skipped for the remote call but their
    /// positions in the result are preserved as zero-vectors so the returned
    /// `Vec` always has the same length as `texts`.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Build a list of (original_index, trimmed_text) for non-blank entries.
        let live: Vec<(usize, String)> = texts
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let trimmed = t.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some((i, trimmed))
                }
            })
            .collect();

        if live.is_empty() {
            // All entries were blank — return zero-vectors.
            return Ok(vec![Vec::new(); texts.len()]);
        }

        let input: Vec<String> = live.iter().map(|(_, t)| t.clone()).collect();

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] sending {} text(s) to ollama model={} ({} blank skipped)",
            input.len(), self.model, texts.len() - input.len()
        );

        let resp = self
            .http_client()
            .post(self.embed_url()?)
            .json(&OllamaEmbedRequest {
                model: self.model.clone(),
                input: input.clone(),
            })
            .send()
            .await
            .map_err(|e| {
                let message = format!(
                    "ollama embed request failed (is Ollama running at {}?): {e}",
                    self.base_url
                );
                crate::core::observability::report_error(
                    message.as_str(),
                    "embeddings",
                    "ollama_embed",
                    &[("model", self.model.as_str()), ("failure", "transport")],
                );
                anyhow::anyhow!(message)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let status_str = status.as_u16().to_string();
            let body = resp.text().await.unwrap_or_default();
            let detail = body.trim();
            let message = format!(
                "ollama embed failed with status {status}{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            );
            crate::core::observability::report_error(
                message.as_str(),
                "embeddings",
                "ollama_embed",
                &[
                    ("model", self.model.as_str()),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
            anyhow::bail!(message);
        }

        let payload: OllamaEmbedResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("ollama embed response parse failed: {e}"))?;

        // Validate response count matches what we sent.
        if payload.embeddings.len() != input.len() {
            anyhow::bail!(
                "ollama embed count mismatch: sent {} texts, got {} embeddings",
                input.len(),
                payload.embeddings.len()
            );
        }

        // Validate dimensions on every returned vector.
        for (i, vec) in payload.embeddings.iter().enumerate() {
            if vec.len() != self.dims {
                anyhow::bail!(
                    "ollama embed dimension mismatch at index {i}: expected {}, got {}",
                    self.dims,
                    vec.len()
                );
            }
        }

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] received {} embeddings, dims={}",
            payload.embeddings.len(),
            self.dims
        );

        // Reconstruct full-length result with zero-vectors for blank positions.
        let mut result = vec![Vec::new(); texts.len()];
        for ((orig_idx, _), embedding) in live.iter().zip(payload.embeddings.into_iter()) {
            result[*orig_idx] = embedding;
        }

        Ok(result)
    }
}

#[cfg(test)]
#[path = "ollama_tests.rs"]
mod tests;
