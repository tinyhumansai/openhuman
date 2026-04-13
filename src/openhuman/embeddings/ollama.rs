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
        Self::new(DEFAULT_OLLAMA_URL, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_DIMENSIONS)
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
    /// Returns one embedding vector per input text. If Ollama is not running
    /// or the model is not available, returns an error (the caller can
    /// decide whether to fall back to another provider).
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let input: Vec<String> = texts
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        if input.is_empty() {
            return Ok(Vec::new());
        }

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] sending {} text(s) to ollama model={}", input.len(), self.model
        );

        let resp = self
            .http_client()
            .post(self.embed_url())
            .json(&OllamaEmbedRequest {
                model: self.model.clone(),
                input,
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

        if payload.embeddings.is_empty() {
            anyhow::bail!("ollama embed returned no embeddings");
        }

        tracing::debug!(
            target: "embeddings.ollama",
            "[embeddings] received {} embeddings, dims={}",
            payload.embeddings.len(),
            payload.embeddings.first().map(|v| v.len()).unwrap_or(0)
        );

        Ok(payload.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let p = OllamaEmbedding::default();
        assert_eq!(p.base_url, DEFAULT_OLLAMA_URL);
        assert_eq!(p.model, DEFAULT_OLLAMA_MODEL);
        assert_eq!(p.dims, DEFAULT_OLLAMA_DIMENSIONS);
    }

    #[test]
    fn name_is_ollama() {
        let p = OllamaEmbedding::default();
        assert_eq!(p.name(), "ollama");
    }

    #[test]
    fn custom_url() {
        let p = OllamaEmbedding::new("http://gpu-box:11434/", "mxbai-embed-large", 1024);
        assert_eq!(p.base_url, "http://gpu-box:11434");
        assert_eq!(p.model, "mxbai-embed-large");
        assert_eq!(p.dims, 1024);
    }

    #[test]
    fn empty_values_use_defaults() {
        let p = OllamaEmbedding::new("", "", 0);
        assert_eq!(p.base_url, DEFAULT_OLLAMA_URL);
        assert_eq!(p.model, DEFAULT_OLLAMA_MODEL);
        assert_eq!(p.dims, DEFAULT_OLLAMA_DIMENSIONS);
    }

    #[test]
    fn embed_url_format() {
        let p = OllamaEmbedding::default();
        assert_eq!(p.embed_url(), "http://localhost:11434/api/embed");
    }

    #[tokio::test]
    async fn empty_input_returns_empty() {
        let p = OllamaEmbedding::default();
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }
}
