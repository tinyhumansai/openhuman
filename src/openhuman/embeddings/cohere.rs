//! Cohere embedding provider — direct API access with user's own key.
//!
//! Cohere's `/v2/embed` endpoint uses a slightly different contract than
//! OpenAI: `texts` instead of `input`, `embedding_types` instead of
//! `encoding_format`, and the response nests embeddings inside
//! `embeddings.float`. This module implements the Cohere-native wire
//! format.

use async_trait::async_trait;

use super::retry_after::{backoff_ms_for_attempt, MAX_429_RETRIES};
use super::EmbeddingProvider;

pub const COHERE_API_BASE: &str = "https://api.cohere.com";
pub const COHERE_DEFAULT_MODEL: &str = "embed-english-v3.0";
pub const COHERE_DEFAULT_DIMS: usize = 1024;

pub struct CohereEmbedding {
    api_key: String,
    model: String,
    dims: usize,
    base_url: String,
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
            base_url: COHERE_API_BASE.to_string(),
        }
    }

    /// Test-only base URL override.  OpenAI's base URL is constructor-injected
    /// since its `new()` already takes one; Cohere historically hardcoded
    /// `COHERE_API_BASE`, so this builder fills the gap for the 429 backoff
    /// tests.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
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

    /// Sends a POST request to the Cohere embed API.
    ///
    /// On 429 (Too Many Requests) or 503 (Service Unavailable) the call is
    /// retried up to `MAX_429_RETRIES` times with exponential backoff.  When
    /// the server supplies a `Retry-After` header its value (delta-seconds) is
    /// preferred over the computed backoff.  After all retries are exhausted the
    /// canonical error message is returned so the `TransientUpstreamHttp`
    /// classifier in `core::observability` demotes it to a warning breadcrumb
    /// instead of a Sentry error event.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/v2/embed", self.base_url);

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

        // Retry loop: handles 429 Too Many Requests and 503 Service Unavailable
        // with Retry-After–aware exponential backoff.
        for attempt in 0..=MAX_429_RETRIES {
            // Proactively gate every outbound attempt (initial + retries) against
            // the per-endpoint rate budget. The chokepoint must sit inside the
            // loop: a single pre-loop acquire would let retried 429/503 attempts
            // bypass token consumption and let concurrent callers blow past the
            // cap, ironically triggering more 429s. Token consumption tracks the
            // number of HTTP attempts (1 + retries actually executed). Loopback
            // endpoints are exempt (see `rate_limit`).
            super::rate_limit::acquire_embedding_slot(&self.base_url).await;

            let resp = self
                .http_client()
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            // Retry on 429 and 503 — both can carry a Retry-After header.
            let is_retryable = status.as_u16() == 429 || status.as_u16() == 503;

            if is_retryable && attempt < MAX_429_RETRIES {
                // Read Retry-After before consuming the body.
                let retry_after_val = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_owned());

                let body_text = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    target: "embeddings.cohere",
                    "[embeddings] cohere {} body on retry: {body_text}",
                    status.as_u16()
                );

                let delay_ms = backoff_ms_for_attempt(attempt, retry_after_val.as_deref());

                tracing::debug!(
                    target: "embeddings.cohere",
                    "[embeddings] cohere {}, retrying in {}ms (attempt {}/{})",
                    status.as_u16(), delay_ms, attempt + 1, MAX_429_RETRIES
                );

                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            if !status.is_success() {
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

            return Ok(embeddings);
        }

        // The loop always exits via `return Ok(...)`, `bail!(...)`, or
        // `continue`; this point is structurally unreachable.  On the final
        // attempt (`attempt == MAX_429_RETRIES`) the retryable guard is false
        // and execution falls into the non-2xx branch above, which bails with
        // the body-bearing format "Cohere embed API error (429 ...): <body>" —
        // that format preserves the "(429 " substring required by the
        // TransientUpstreamHttp classifier in core::observability.
        unreachable!("cohere embed retry loop must exit via return or bail")
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

    // ── 429 backoff tests ──────────────────────────────────────

    use axum::{http::StatusCode, routing::post, Router};
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };

    async fn start_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    /// Cohere 429 then success — verifies retry recovers.
    ///
    /// The mock returns 429 (with Retry-After: 0 for zero real-wall-clock delay)
    /// twice, then 200 on the third call.  The real `CohereEmbedding::embed` is
    /// driven via `with_base_url` pointing at the axum mock server.
    #[tokio::test]
    async fn cohere_embed_429_then_success() {
        let counter = Arc::new(Mutex::new(0u32));
        let counter_clone = counter.clone();

        let app = Router::new().route(
            "/v2/embed",
            post(move || {
                let counter = counter_clone.clone();
                async move {
                    let mut n = counter.lock().unwrap();
                    *n += 1;
                    if *n <= 2 {
                        axum::response::Response::builder()
                            .status(StatusCode::TOO_MANY_REQUESTS)
                            .header("Retry-After", "0")
                            .body(axum::body::Body::from(r#"{"message":"rate limited"}"#))
                            .unwrap()
                    } else {
                        axum::response::Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(axum::body::Body::from(
                                r#"{"embeddings":{"float":[[0.1,0.2]]}}"#,
                            ))
                            .unwrap()
                    }
                }
            }),
        );

        let base_url = start_mock(app).await;
        let p = CohereEmbedding::new("test-key", "embed-english-v3.0", 2).with_base_url(&base_url);

        let result = p.embed(&["hello"]).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(*counter.lock().unwrap(), 3, "should have taken 3 requests");
    }

    /// Cohere 429 indefinitely — verify bail with canonical message after retry
    /// cap, and that exactly `MAX_429_RETRIES + 1` requests were made.
    #[tokio::test]
    async fn cohere_embed_429_indefinite_bails_after_cap() {
        let counter = Arc::new(Mutex::new(0u32));
        let counter_clone = counter.clone();

        let app = Router::new().route(
            "/v2/embed",
            post(move || {
                let counter = counter_clone.clone();
                async move {
                    let mut n = counter.lock().unwrap();
                    *n += 1;
                    axum::response::Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header("Retry-After", "0")
                        .body(axum::body::Body::from(r#"{"message":"always limited"}"#))
                        .unwrap()
                }
            }),
        );

        let base_url = start_mock(app).await;
        let p = CohereEmbedding::new("test-key", "embed-english-v3.0", 2).with_base_url(&base_url);

        let err = p.embed(&["hello"]).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("429"),
            "should contain 429 in error message: {msg}"
        );
        // MAX_429_RETRIES retries + 1 initial = MAX_429_RETRIES + 1 total requests
        assert_eq!(
            *counter.lock().unwrap(),
            MAX_429_RETRIES + 1,
            "should make exactly MAX_429_RETRIES+1 requests"
        );
    }
}
