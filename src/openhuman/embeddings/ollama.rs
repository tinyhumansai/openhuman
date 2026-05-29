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

    /// Sends a single text to Ollama and returns either the embedding, or
    /// `None` if Ollama produced the NaN-encoding 500 wire shape for that
    /// individual input. Any other failure (transport error, non-2xx without
    /// NaN signature, malformed JSON, dimension mismatch, count mismatch)
    /// surfaces as an `Err` so genuine bugs are still loud.
    ///
    /// Used by [`Self::embed_per_text_fallback`] to recover a batch that
    /// failed wholesale on NaN — see TAURI-RUST-AZ.
    async fn embed_one_with_nan_recovery(&self, text: &str) -> anyhow::Result<Option<Vec<f32>>> {
        let resp = self
            .http_client()
            .post(self.embed_url()?)
            .json(&OllamaEmbedRequest {
                model: self.model.clone(),
                input: vec![text.to_string()],
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
            // Per-text NaN: substitute empty vector, log once.
            if status.as_u16() == 500 && is_nan_encode_error(&body) {
                tracing::warn!(
                    target: "embeddings.ollama",
                    "[embeddings] ollama produced NaN for a single text (model={}); \
                     substituting empty embedding (downstream blob will be 0 bytes)",
                    self.model
                );
                return Ok(None);
            }
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
        if payload.embeddings.len() != 1 {
            anyhow::bail!(
                "ollama embed count mismatch: sent 1 text, got {} embeddings",
                payload.embeddings.len()
            );
        }
        let v = payload.embeddings.into_iter().next().unwrap();
        if v.len() != self.dims {
            anyhow::bail!(
                "ollama embed dimension mismatch: expected {}, got {}",
                self.dims,
                v.len()
            );
        }
        Ok(Some(v))
    }

    /// Recovery path invoked when a batch request fails with the Ollama
    /// NaN-encoding 500 wire shape. Re-sends each live input one at a time;
    /// per-text NaN failures are filled with `Vec::new()` so the overall
    /// result vector still has length `total_len` and aligns with the
    /// caller's original `texts` slice (same convention as blank inputs).
    async fn embed_per_text_fallback(
        &self,
        total_len: usize,
        live: &[(usize, String)],
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut result = vec![Vec::new(); total_len];
        let mut nan_substituted = 0usize;
        for (orig_idx, text) in live {
            match self.embed_one_with_nan_recovery(text).await? {
                Some(v) => result[*orig_idx] = v,
                None => nan_substituted += 1,
            }
        }
        tracing::warn!(
            target: "embeddings.ollama",
            "[embeddings] ollama per-text fallback complete: {} of {} inputs returned NaN and \
             were substituted with empty embeddings",
            nan_substituted,
            live.len()
        );
        Ok(result)
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

/// Detects the Ollama-side NaN-encoding 500 wire shape:
///   `{"error":"failed to encode response: json: unsupported value: NaN"}`
///
/// Ollama produces NaN for some inputs (model bug / numerically degenerate
/// token sequences) and then fails to encode the response as JSON. One bad
/// input poisons the entire batch.
///
/// See TAURI-RUST-AZ on Sentry.
fn is_nan_encode_error(body: &str) -> bool {
    body.to_ascii_lowercase().contains("unsupported value: nan")
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    fn name(&self) -> &str {
        "ollama"
    }

    fn model_id(&self) -> &str {
        &self.model
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
                crate::core::observability::report_error_or_expected(
                    &message,
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

            // TAURI-RUST-AZ: Ollama returns 500 `unsupported value: NaN` when the
            // model produces NaN for some input in the batch. One bad input
            // poisons the whole batch under the default code path. Recover by
            // re-sending each live input individually; per-text NaN failures
            // are replaced with an empty embedding (same convention used for
            // blank inputs above), so the rest of the batch still succeeds.
            if status.as_u16() == 500 && is_nan_encode_error(&body) {
                tracing::warn!(
                    target: "embeddings.ollama",
                    "[embeddings] ollama returned NaN-encoding 500 for batch of {} (model={}); \
                     falling back to per-text requests",
                    live.len(),
                    self.model
                );
                crate::core::observability::report_error_or_expected(
                    &message,
                    "embeddings",
                    "ollama_embed",
                    &[
                        ("model", self.model.as_str()),
                        ("status", status_str.as_str()),
                        ("failure", "nan_batch_recovered"),
                    ],
                );
                // Single-text NaN: re-issuing the same request would only
                // reproduce the failure. Skip the wasted round-trip and
                // substitute an empty embedding directly.
                if live.len() == 1 {
                    return Ok(vec![Vec::new(); texts.len()]);
                }
                return self.embed_per_text_fallback(texts.len(), &live).await;
            }

            crate::core::observability::report_error_or_expected(
                &message,
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
