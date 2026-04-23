use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Returns vectors in the same order as `texts`. All vectors share `dim()`.
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
}

#[derive(Clone)]
pub struct HostedEmbedder {
    base_url: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl HostedEmbedder {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url,
            api_key,
            model,
            http,
        }
    }
}

#[derive(Serialize)]
struct EmbedReq<'a> {
    input: &'a [&'a str],
    model: &'a str,
}

#[derive(Deserialize)]
struct EmbedRespItem {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedResp {
    data: Vec<EmbedRespItem>,
}

#[async_trait]
impl Embedder for HostedEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&EmbedReq {
                input: texts,
                model: &self.model,
            })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResp>()
            .await?;
        let mut data = resp.data;
        if data.len() != texts.len() {
            anyhow::bail!(
                "embeddings response length {} != input {}",
                data.len(),
                texts.len()
            );
        }
        // Keep order by index in case the server returns out of order.
        data.sort_by_key(|d| d.index);
        let dim = self.dim();
        for (i, d) in data.iter().enumerate() {
            if d.index != i {
                anyhow::bail!(
                    "embeddings response has missing/duplicate index at position {i} (got index {})",
                    d.index
                );
            }
            if d.embedding.len() != dim {
                anyhow::bail!(
                    "embedding at index {i} has dim {} (expected {dim})",
                    d.embedding.len()
                );
            }
        }
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    fn dim(&self) -> usize {
        1536
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[tokio::test]
    async fn hosted_embedder_calls_openai_compatible_endpoint() {
        let server = MockServer::start_async().await;
        let body = serde_json::json!({
            "data": [
                {"index": 0, "embedding": vec![0.1_f32; 1536]},
                {"index": 1, "embedding": vec![0.2_f32; 1536]},
            ],
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 4, "total_tokens": 4}
        });
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/v1/embeddings");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(body);
            })
            .await;

        let emb = HostedEmbedder::new(
            format!("{}/v1", server.base_url()),
            "test-key".into(),
            "text-embedding-3-small".into(),
        );
        let out = emb.embed_batch(&["hello", "world"]).await.expect("embed");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 1536);
        assert!((out[0][0] - 0.1).abs() < 1e-6);
        mock.assert_async().await;
    }
}
