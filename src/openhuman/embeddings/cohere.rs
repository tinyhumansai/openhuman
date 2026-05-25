//! Cohere embedding provider — direct API access with user's own key.
//!
//! Cohere's `/v2/embed` endpoint uses a slightly different contract than
//! OpenAI: `texts` instead of `input`, `embedding_types` instead of
//! `encoding_format`, and the response nests embeddings inside
//! `embeddings.float`. This module implements the Cohere-native wire
//! format.

use async_trait::async_trait;

use super::EmbeddingProvider;

pub const COHERE_API_BASE: &str = "https://api.cohere.com";
pub const COHERE_DEFAULT_MODEL: &str = "embed-english-v3.0";
pub const COHERE_DEFAULT_DIMS: usize = 1024;

pub struct CohereEmbedding {
    api_key: String,
    model: String,
    dims: usize,
}

impl CohereEmbedding {
    pub fn new(api_key: &str, model: &str, dims: usize) -> Self {
        let model = if model.is_empty() {
            COHERE_DEFAULT_MODEL.to_string()
        } else {
            model.to_string()
        };
        let dims = if dims == 0 { COHERE_DEFAULT_DIMS } else { dims };

        Self {
            api_key: api_key.to_string(),
            model,
            dims,
        }
    }

    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("embeddings.cohere")
    }
}

#[derive(serde::Deserialize)]
struct CohereEmbedResponse {
    embeddings: CohereEmbeddings,
}

#[derive(serde::Deserialize)]
struct CohereEmbeddings {
    float: Vec<Vec<f32>>,
}

#[async_trait]
impl EmbeddingProvider for CohereEmbedding {
    fn name(&self) -> &str {
        "cohere"
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

        super::rate_limit::acquire_embedding_slot(COHERE_API_BASE).await;

        let url = format!("{COHERE_API_BASE}/v2/embed");

        tracing::debug!(
            target: "embeddings.cohere",
            "[cohere] embed: model={}, count={}", self.model, texts.len()
        );

        let body = serde_json::json!({
            "model": self.model,
            "texts": texts,
            "input_type": "search_document",
            "embedding_types": ["float"],
        });

        let resp = self
            .http_client()
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let message = format!("Cohere embed API error ({status}): {text}");
            crate::core::observability::report_error_or_expected(
                &message,
                "embeddings",
                "cohere_embed",
                &[("model", self.model.as_str()), ("failure", "non_2xx")],
            );
            anyhow::bail!(message);
        }

        let payload: CohereEmbedResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Cohere embed response parse failed: {e}"))?;

        let embeddings = payload.embeddings.float;

        if embeddings.len() != texts.len() {
            anyhow::bail!(
                "Cohere embed count mismatch: sent {} texts, got {} embeddings",
                texts.len(),
                embeddings.len()
            );
        }

        for (i, vec) in embeddings.iter().enumerate() {
            if self.dims > 0 && vec.len() != self.dims {
                anyhow::bail!(
                    "Cohere embed dimension mismatch at index {i}: expected {}, got {}",
                    self.dims,
                    vec.len()
                );
            }
        }

        tracing::debug!(
            target: "embeddings.cohere",
            "[cohere] embed success: model={}, count={}, dims={}",
            self.model, embeddings.len(),
            embeddings.first().map(|v| v.len()).unwrap_or(0)
        );

        Ok(embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_defaults() {
        let p = CohereEmbedding::new("test-key", "", 0);
        assert_eq!(p.name(), "cohere");
        assert_eq!(p.model_id(), COHERE_DEFAULT_MODEL);
        assert_eq!(p.dimensions(), COHERE_DEFAULT_DIMS);
    }

    #[test]
    fn custom_model() {
        let p = CohereEmbedding::new("k", "embed-multilingual-v3.0", 1024);
        assert_eq!(p.model_id(), "embed-multilingual-v3.0");
    }

    #[test]
    fn signature_format() {
        let p = CohereEmbedding::new("k", "embed-english-v3.0", 1024);
        assert_eq!(
            p.signature(),
            "provider=cohere;model=embed-english-v3.0;dims=1024"
        );
    }

    #[tokio::test]
    async fn embed_empty_returns_empty() {
        let p = CohereEmbedding::new("k", "", 0);
        assert!(p.embed(&[]).await.unwrap().is_empty());
    }
}
