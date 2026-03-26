use async_trait::async_trait;

/// Trait for embedding providers — convert text to vectors
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider name
    fn name(&self) -> &str;

    /// Embedding dimensions
    fn dimensions(&self) -> usize;

    /// Embed a batch of texts into vectors
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Embed a single text
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }
}

// ── Noop provider (keyword-only fallback) ────────────────────

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

// ── OpenAI-compatible embedding provider ─────────────────────

pub struct OpenAiEmbedding {
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
}

impl OpenAiEmbedding {
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dims,
        }
    }

    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("memory.embeddings")
    }

    fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    fn has_embeddings_endpoint(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        url.path().trim_end_matches('/').ends_with("/embeddings")
    }

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

pub fn create_embedding_provider(
    provider: &str,
    api_key: Option<&str>,
    model: &str,
    dims: usize,
) -> Box<dyn EmbeddingProvider> {
    match provider {
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
