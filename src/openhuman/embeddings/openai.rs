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
mod tests {
    use super::*;
    use axum::{
        extract::Json,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::net::SocketAddr;

    async fn start_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    // ── Constructor & URL building ──────────────────────────

    #[test]
    fn trailing_slash_stripped() {
        let p = OpenAiEmbedding::new("https://api.openai.com/", "key", "model", 1536);
        assert_eq!(p.base_url, "https://api.openai.com");
    }

    #[test]
    fn dimensions_custom() {
        let p = OpenAiEmbedding::new("http://localhost", "k", "m", 384);
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn accessors() {
        let p = OpenAiEmbedding::new("http://x", "k", "m", 1);
        assert_eq!(p.base_url(), "http://x");
        assert_eq!(p.model(), "m");
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn url_standard_openai() {
        let p = OpenAiEmbedding::new("https://api.openai.com", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn url_base_with_v1_no_duplicate() {
        let p = OpenAiEmbedding::new("https://api.example.com/v1", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
    }

    #[test]
    fn url_non_v1_api_path() {
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
    fn url_already_ends_with_embeddings() {
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

    #[test]
    fn url_already_ends_with_embeddings_trailing_slash() {
        let p = OpenAiEmbedding::new(
            "https://api.example.com/v1/embeddings/",
            "key",
            "model",
            1536,
        );
        assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
    }

    #[test]
    fn url_root_only() {
        let p = OpenAiEmbedding::new("http://localhost:8080", "k", "m", 1);
        assert_eq!(p.embeddings_url(), "http://localhost:8080/v1/embeddings");
    }

    #[test]
    fn url_root_with_trailing_slash() {
        let p = OpenAiEmbedding::new("http://localhost:8080/", "k", "m", 1);
        assert_eq!(p.embeddings_url(), "http://localhost:8080/v1/embeddings");
    }

    #[test]
    fn has_explicit_api_path_invalid_url() {
        let p = OpenAiEmbedding::new("not-a-url", "k", "m", 1);
        assert!(!p.has_explicit_api_path());
    }

    #[test]
    fn has_embeddings_endpoint_invalid_url() {
        let p = OpenAiEmbedding::new("not-a-url", "k", "m", 1);
        assert!(!p.has_embeddings_endpoint());
    }

    // ── embed — empty input ─────────────────────────────────

    #[tokio::test]
    async fn empty_input_returns_empty() {
        let p = OpenAiEmbedding::new("http://unused", "k", "m", 1);
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    // ── embed — success ─────────────────────────────────────

    #[tokio::test]
    async fn embed_success_single() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [0.1, 0.2, 0.3] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "test-key", "test-model", 3);

        let result = p.embed(&["hello"]).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec![0.1_f32, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn embed_success_batch() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [
                        { "embedding": [1.0, 2.0] },
                        { "embedding": [3.0, 4.0] }
                    ]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 2);

        let result = p.embed(&["a", "b"]).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[1], vec![3.0_f32, 4.0]);
    }

    #[tokio::test]
    async fn embed_sends_auth_header() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(
                |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                    let auth = headers.get("Authorization").unwrap().to_str().unwrap();
                    assert_eq!(auth, "Bearer my-secret-key");
                    assert_eq!(body["model"], "text-embedding-3-small");
                    Json(serde_json::json!({
                        "data": [{ "embedding": [1.0] }]
                    }))
                },
            ),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "my-secret-key", "text-embedding-3-small", 1);

        p.embed(&["test"]).await.unwrap();
    }

    #[tokio::test]
    async fn embed_skips_auth_header_when_key_empty() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|headers: HeaderMap| async move {
                // No Authorization header should be present.
                assert!(
                    headers.get("Authorization").is_none(),
                    "should not send auth header when key is empty"
                );
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "", "m", 1);

        p.embed(&["test"]).await.unwrap();
    }

    // ── embed — error paths ─────────────────────────────────

    #[tokio::test]
    async fn embed_server_error() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "rate limited") }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 1);

        let err = p.embed(&["hi"]).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"), "status: {msg}");
        assert!(msg.contains("rate limited"), "body: {msg}");
    }

    #[tokio::test]
    async fn embed_missing_data_field() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async { Json(serde_json::json!({ "result": "ok" })) }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 1);

        let err = p.embed(&["hi"]).await.unwrap_err();
        assert!(err.to_string().contains("missing 'data'"));
    }

    #[tokio::test]
    async fn embed_missing_embedding_field_in_item() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "index": 0 }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 1);

        let err = p.embed(&["hi"]).await.unwrap_err();
        assert!(err.to_string().contains("missing 'embedding'"));
    }

    #[tokio::test]
    async fn embed_non_numeric_value_errors() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0, "not_a_number", 3.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 3);

        let err = p.embed(&["hi"]).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non-numeric"), "msg: {msg}");
    }

    #[tokio::test]
    async fn embed_count_mismatch() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 1);

        let err = p.embed(&["a", "b"]).await.unwrap_err();
        assert!(err.to_string().contains("count mismatch"));
    }

    #[tokio::test]
    async fn embed_dimension_mismatch() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0, 2.0, 3.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 2);

        let err = p.embed(&["hi"]).await.unwrap_err();
        assert!(err.to_string().contains("dimension mismatch"));
    }

    #[tokio::test]
    async fn embed_malformed_json() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async { (StatusCode::OK, "not json") }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 1);

        let err = p.embed(&["hi"]).await.unwrap_err();
        assert!(err.is::<reqwest::Error>());
    }

    #[tokio::test]
    async fn embed_connection_refused() {
        let p = OpenAiEmbedding::new("http://127.0.0.1:1", "k", "m", 1);
        let err = p.embed(&["hi"]).await.unwrap_err();
        assert!(err.is::<reqwest::Error>());
    }

    // ── embed_one (trait default) ───────────────────────────

    #[tokio::test]
    async fn embed_one_success() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [9.0, 8.0, 7.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&url, "k", "m", 3);

        let vec = p.embed_one("test").await.unwrap();
        assert_eq!(vec, vec![9.0_f32, 8.0, 7.0]);
    }

    // ── URL building — custom endpoint ──────────────────────

    #[tokio::test]
    async fn embed_with_explicit_api_path() {
        let app = Router::new().route(
            "/custom/api/embeddings",
            post(|| async {
                Json(serde_json::json!({
                    "data": [{ "embedding": [1.0] }]
                }))
            }),
        );
        let url = start_mock(app).await;
        let p = OpenAiEmbedding::new(&format!("{url}/custom/api"), "k", "m", 1);

        let result = p.embed(&["test"]).await.unwrap();
        assert_eq!(result.len(), 1);
    }
}
