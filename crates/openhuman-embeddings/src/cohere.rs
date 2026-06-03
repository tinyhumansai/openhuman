//! Cohere embedding provider — direct API access with user's own key.

use async_trait::async_trait;

use crate::retry_after::{backoff_ms_for_attempt, MAX_429_RETRIES};
use crate::EmbeddingProvider;

pub const COHERE_API_BASE: &str = "https://api.cohere.com";
pub const COHERE_DEFAULT_MODEL: &str = "embed-english-v3.0";
pub const COHERE_DEFAULT_DIMS: usize = 1024;

pub struct CohereEmbedding {
    api_key: String,
    model: String,
    dims: usize,
    base_url: String,
    client: reqwest::Client,
}

impl CohereEmbedding {
    pub fn new(api_key: &str, model: &str, dims: usize) -> Self {
        let model = if model.is_empty() { COHERE_DEFAULT_MODEL.to_string() } else { model.to_string() };
        let dims = if dims == 0 { COHERE_DEFAULT_DIMS } else { dims };
        Self { api_key: api_key.to_string(), model, dims, base_url: COHERE_API_BASE.to_string(), client: reqwest::Client::new() }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into().trim().trim_end_matches('/').to_string();
        self
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[derive(serde::Deserialize)]
struct CohereEmbedResponse { embeddings: CohereEmbeddings }
#[derive(serde::Deserialize)]
struct CohereEmbeddings { float: Vec<Vec<f32>> }

#[async_trait]
impl EmbeddingProvider for CohereEmbedding {
    fn name(&self) -> &str { "cohere" }
    fn model_id(&self) -> &str { &self.model }
    fn dimensions(&self) -> usize { self.dims }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() { return Ok(Vec::new()); }

        let url = format!("{}/v2/embed", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "texts": texts,
            "input_type": "search_document",
            "embedding_types": ["float"],
        });

        for attempt in 0..=MAX_429_RETRIES {
            crate::rate_limit::acquire_embedding_slot(&self.base_url).await;

            let resp = self.client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            let is_retryable = status.as_u16() == 429 || status.as_u16() == 503;

            if is_retryable && attempt < MAX_429_RETRIES {
                let retry_after_val = resp.headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_owned());
                let _ = resp.text().await;
                let delay_ms = backoff_ms_for_attempt(attempt, retry_after_val.as_deref());
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                let message = format!("Cohere embed API error ({status}): {text}");
                tracing::warn!(target: "embeddings", model = %self.model, "{message}");
                anyhow::bail!(message);
            }

            let payload: CohereEmbedResponse = resp.json().await
                .map_err(|e| anyhow::anyhow!("Cohere response parse failed: {e}"))?;
            let embeddings = payload.embeddings.float;

            if embeddings.len() != texts.len() {
                anyhow::bail!("Cohere count mismatch: sent {}, got {}", texts.len(), embeddings.len());
            }
            for (i, vec) in embeddings.iter().enumerate() {
                if self.dims > 0 && vec.len() != self.dims {
                    anyhow::bail!("Cohere dims mismatch at {i}: expected {}, got {}", self.dims, vec.len());
                }
            }
            return Ok(embeddings);
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let p = CohereEmbedding::new("k", "", 0);
        assert_eq!(p.name(), "cohere");
        assert_eq!(p.model_id(), COHERE_DEFAULT_MODEL);
    }

    #[tokio::test]
    async fn embed_empty() {
        let p = CohereEmbedding::new("k", "", 0);
        assert!(p.embed(&[]).await.unwrap().is_empty());
    }
}
