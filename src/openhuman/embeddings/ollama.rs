//! Ollama-based embedding provider.
//!
//! Calls the local Ollama server's `/api/embed` endpoint for embeddings.
//! This is the preferred local provider: Ollama handles model management,
//! quantization, and GPU acceleration (Metal on macOS, CUDA on Linux/Windows).
//!
//! Default model: `nomic-embed-text:latest` (768 dimensions).

use async_trait::async_trait;

use super::EmbeddingProvider;

/// Default Ollama base URL.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Default embedding model for Ollama.
pub const DEFAULT_OLLAMA_MODEL: &str = "nomic-embed-text:latest";

/// Default dimensions for nomic-embed-text.
pub const DEFAULT_OLLAMA_DIMENSIONS: usize = 768;

/// Embedding provider backed by a local Ollama instance.
///
/// Ollama must be running and have the configured model pulled.
/// On first embed call, if the model isn't available, Ollama will
/// auto-pull it (this may take a moment on first use).
pub struct OllamaEmbedding {
    base_url: String,
    model: String,
    dims: usize,
}

impl OllamaEmbedding {
    /// Creates a new Ollama embedding provider.
    ///
    /// - `base_url`: Ollama server URL (default: `http://localhost:11434`)
    /// - `model`: Model name (default: `nomic-embed-text:latest`)
    /// - `dims`: Expected embedding dimensions (default: 768)
    pub fn new(base_url: &str, model: &str, dims: usize) -> Self {
        let base_url = if base_url.trim().is_empty() {
            DEFAULT_OLLAMA_URL.to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        let model = if model.trim().is_empty() {
            DEFAULT_OLLAMA_MODEL.to_string()
        } else {
            model.trim().to_string()
        };
        let dims = if dims == 0 {
            DEFAULT_OLLAMA_DIMENSIONS
        } else {
            dims
        };

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] OllamaEmbedding created: url={base_url}, model={model}, dims={dims}"
        );

        Self {
            base_url,
            model,
            dims,
        }
    }

    /// Creates a provider with all defaults.
    pub fn default() -> Self {
        Self::new(
            DEFAULT_OLLAMA_URL,
            DEFAULT_OLLAMA_MODEL,
            DEFAULT_OLLAMA_DIMENSIONS,
        )
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

    /// The embed endpoint URL.
    fn embed_url(&self) -> String {
        format!("{}/api/embed", self.base_url)
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
            .post(self.embed_url())
            .json(&OllamaEmbedRequest {
                model: self.model.clone(),
                input: input.clone(),
            })
            .send()
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "ollama embed request failed (is Ollama running at {}?): {e}",
                    self.base_url
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let detail = body.trim();
            anyhow::bail!(
                "ollama embed failed with status {status}{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            );
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
