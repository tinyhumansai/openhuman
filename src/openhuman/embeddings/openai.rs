//! OpenAI-compatible embedding provider.
//!
//! Works with OpenAI, LocalAI, Ollama, and any endpoint that implements the
//! `POST /v1/embeddings` contract.

use async_trait::async_trait;

use super::EmbeddingProvider;

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

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured model name.
    pub fn model(&self) -> &str {
        &self.model
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

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Sends a POST request to the embedding API.
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

        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let mut req = self
            .http_client()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body);

        // Only set Authorization header when an API key is configured.
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::debug!(
                target: "openai::embed",
                "[openai] embed error: status={status}, body={text}"
            );
            anyhow::bail!("Embedding API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding response: missing 'data'"))?;

        // Validate that the response count matches the input count.
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

            // Validate dimensions.
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

        Ok(embeddings)
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
