//! OpenAI-compatible embedding provider.
//!
//! Works with OpenAI, LocalAI, Ollama, and any endpoint that implements the
//! `POST /v1/embeddings` contract.

use async_trait::async_trait;

use crate::retry_after::{backoff_ms_for_attempt, MAX_429_RETRIES};
use crate::EmbeddingProvider;

/// Embedding provider for OpenAI and compatible APIs (e.g., LocalAI, Ollama).
pub struct OpenAiEmbedding {
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
    /// When true, send `"dimensions": dims` in the request body. OpenAI's
    /// `text-embedding-3-*` models honour this (Matryoshka — e.g. 3-large can
    /// return 1024 instead of its native 3072). Off by default so providers
    /// that don't accept the field — Voyage (uses `output_dimension`), Cohere,
    /// LocalAI/Ollama — keep working unchanged. Set via
    /// [`Self::with_send_dimensions`] for the OpenAI / custom-OpenAI paths.
    send_dimensions: bool,
    /// HTTP client for making requests. Inject a pre-configured client (e.g.
    /// with proxy settings) via [`Self::with_client`].
    client: reqwest::Client,
}

impl OpenAiEmbedding {
    /// Creates a new OpenAI-style provider.
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dims,
            send_dimensions: false,
            client: reqwest::Client::new(),
        }
    }

    /// Opt into sending the OpenAI `dimensions` request parameter so a
    /// reducible model (`text-embedding-3-large` / `-3-small`) returns exactly
    /// `dims` floats instead of its native size. Returns `self` for builder chaining.
    pub fn with_send_dimensions(mut self, send: bool) -> Self {
        self.send_dimensions = send;
        self
    }

    /// Set a custom HTTP client (e.g. with proxy configuration).
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured model name.
    pub fn model(&self) -> &str {
        &self.model
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
    pub fn embeddings_url(&self) -> String {
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

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = self.embeddings_url();

        tracing::debug!(
            target: "openai::embed",
            "[openai] embed: model={}, count={}, url={}",
            self.model, texts.len(), url
        );

        let mut body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });
        if self.send_dimensions && self.dims > 0 {
            body["dimensions"] = serde_json::json!(self.dims);
        }

        for attempt in 0..=MAX_429_RETRIES {
            let mut req = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);

            if !self.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", self.api_key));
            }

            crate::rate_limit::acquire_embedding_slot(&self.base_url).await;

            let resp = req.send().await?;
            let status = resp.status();

            let is_retryable = status.as_u16() == 429 || status.as_u16() == 503;

            if is_retryable && attempt < MAX_429_RETRIES {
                let retry_after_val = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_owned());

                let body_text = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    target: "openai::embed",
                    "[embeddings] openai {} body on retry: {body_text}",
                    status.as_u16()
                );

                let delay_ms = backoff_ms_for_attempt(attempt, retry_after_val.as_deref());

                tracing::debug!(
                    target: "openai::embed",
                    "[embeddings] openai {}, retrying in {}ms (attempt {}/{})",
                    status.as_u16(), delay_ms, attempt + 1, MAX_429_RETRIES
                );

                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    target: "openai::embed",
                    "[openai] embed error: status={status}, body={text}"
                );
                let message = format!("Embedding API error ({status}): {text}");
                tracing::warn!(
                    target: "embeddings",
                    model = %self.model,
                    status = %status.as_u16(),
                    "{message}"
                );
                anyhow::bail!(message);
            }

            let json: serde_json::Value = resp.json().await?;
            let data = json
                .get("data")
                .and_then(|d| d.as_array())
                .ok_or_else(|| anyhow::anyhow!("Invalid embedding response: missing 'data'"))?;

            if data.len() != texts.len() {
                anyhow::bail!(
                    "openai embed count mismatch: sent {} texts, got {} items in 'data'",
                    texts.len(),
                    data.len()
                );
            }

            let mut embeddings = Vec::with_capacity(data.len());
            for (i, item) in data.iter().enumerate() {
                let embedding = item
                    .get("embedding")
                    .and_then(|e| e.as_array())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Invalid embedding item at index {i}: missing 'embedding'")
                    })?;

                let mut vec = Vec::with_capacity(embedding.len());
                for (j, v) in embedding.iter().enumerate() {
                    #[allow(clippy::cast_possible_truncation)]
                    let f = v.as_f64().ok_or_else(|| {
                        anyhow::anyhow!("non-numeric value at data[{i}].embedding[{j}]: {v}")
                    })? as f32;
                    vec.push(f);
                }

                if self.dims > 0 && vec.len() != self.dims {
                    anyhow::bail!(
                        "openai embed dimension mismatch at index {i}: expected {}, got {}",
                        self.dims,
                        vec.len()
                    );
                }

                embeddings.push(vec);
            }

            tracing::debug!(
                target: "openai::embed",
                "[openai] embed success: model={}, count={}, dims={}",
                self.model, embeddings.len(),
                embeddings.first().map(|v| v.len()).unwrap_or(0)
            );

            return Ok(embeddings);
        }

        unreachable!("embed retry loop must exit via return or bail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_url_appends_v1_path() {
        let e = OpenAiEmbedding::new("https://api.openai.com", "", "m", 1536);
        assert_eq!(e.embeddings_url(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embeddings_url_preserves_explicit_path() {
        let e = OpenAiEmbedding::new("https://example.com/api/v1", "", "m", 1536);
        assert_eq!(e.embeddings_url(), "https://example.com/api/v1/embeddings");
    }

    #[test]
    fn embeddings_url_no_double_suffix() {
        let e = OpenAiEmbedding::new("https://example.com/v1/embeddings", "", "m", 1536);
        assert_eq!(e.embeddings_url(), "https://example.com/v1/embeddings");
    }
}
