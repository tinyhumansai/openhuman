//! Ollama-backed embedder for Phase 4 (#710).
//!
//! Posts `{model, prompt}` to `{endpoint}/api/embeddings` and expects
//! `{"embedding": [f32; EMBEDDING_DIM]}` back. Designed for a local
//! `ollama serve` hosting `nomic-embed-text`.
//!
//! This is intentionally a tiny HTTP client — no retry, no pool caching,
//! no streaming. Phase 4 wants the simplest thing that works so we can
//! land embedding end-to-end and iterate once baseline retrieval quality
//! is measurable. Timeouts, parallelism, and caching are explicit
//! follow-ups.

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Embedder, EMBEDDING_DIM};

/// Default Ollama endpoint. Matches the local-install default from the
/// `local_ai` subsystem and the Ollama defaults.
pub const DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Default embedding model — must output exactly [`EMBEDDING_DIM`] dims.
pub const DEFAULT_MODEL: &str = "nomic-embed-text";

/// Default request timeout. Ollama's first-use latency is a few hundred
/// ms on a warm model; 10s absorbs a cold-model load on commodity
/// hardware without stalling ingest on a broken backend.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// HTTP client wrapping a single Ollama endpoint + model pair.
///
/// Cloneable — `reqwest::Client` shares a connection pool under the hood
/// so cloning the wrapper stays cheap across seal / ingest call sites.
#[derive(Clone)]
pub struct OllamaEmbedder {
    endpoint: String,
    model: String,
    #[allow(dead_code)]
    timeout: Duration,
    client: reqwest::Client,
}

impl OllamaEmbedder {
    /// Build a new embedder. `endpoint` is trimmed of trailing slashes
    /// so callers don't have to worry about mixing `http://host` and
    /// `http://host/`. Empty values fall back to the public defaults.
    pub fn new(endpoint: String, model: String, timeout_ms: u64) -> Self {
        let endpoint = if endpoint.trim().is_empty() {
            DEFAULT_ENDPOINT.to_string()
        } else {
            endpoint.trim().trim_end_matches('/').to_string()
        };
        let model = if model.trim().is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            model.trim().to_string()
        };
        let timeout_ms = if timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            timeout_ms
        };
        let timeout = Duration::from_millis(timeout_ms);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            // Falling back to the default client keeps `new` infallible;
            // timeouts will apply per-request via explicit `.timeout()`
            // calls below.
            .unwrap_or_else(|e| {
                log::warn!(
                    "[memory_tree::embed::ollama] failed to build client \
                     with timeout — using default: {e}"
                );
                reqwest::Client::new()
            });
        log::debug!(
            "[memory_tree::embed::ollama] created endpoint={endpoint} \
             model={model} timeout_ms={timeout_ms}"
        );
        Self {
            endpoint,
            model,
            timeout,
            client,
        }
    }

    /// Convenience constructor using all defaults.
    pub fn default_new() -> Self {
        Self::new(String::new(), String::new(), 0)
    }

    fn embed_url(&self) -> String {
        format!("{}/api/embeddings", self.endpoint)
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embedding: Vec<f32>,
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    fn name(&self) -> &'static str {
        "ollama"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        log::debug!(
            "[memory_tree::embed::ollama] embed endpoint={} model={} bytes={}",
            self.endpoint,
            self.model,
            text.len()
        );
        let req = EmbedRequest {
            model: &self.model,
            prompt: text,
        };
        let resp = self
            .client
            .post(self.embed_url())
            .json(&req)
            .timeout(self.timeout)
            .send()
            .await
            .with_context(|| {
                format!(
                    "ollama embeddings request failed (is Ollama running at {}?)",
                    self.endpoint
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "ollama embeddings failed status={status} body={}",
                body.trim()
            );
        }

        let payload: EmbedResponse = resp
            .json()
            .await
            .context("ollama embeddings response parse failed")?;

        if payload.embedding.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "ollama embeddings returned {} dims, expected {EMBEDDING_DIM} \
                 (check model={})",
                payload.embedding.len(),
                self.model
            );
        }

        Ok(payload.embedding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Json, http::StatusCode, routing::post, Router};
    use std::net::SocketAddr;

    /// Spin up a local axum server and return its base URL.
    async fn start_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn fixed_vec(val: f32) -> Vec<f32> {
        vec![val; EMBEDDING_DIM]
    }

    #[test]
    fn defaults_applied_on_empty_input() {
        let e = OllamaEmbedder::new(String::new(), String::new(), 0);
        assert_eq!(e.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(e.model, DEFAULT_MODEL);
        assert_eq!(e.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
    }

    #[test]
    fn trailing_slash_trimmed() {
        let e = OllamaEmbedder::new("http://host:1234/".into(), "m".into(), 1000);
        assert_eq!(e.endpoint, "http://host:1234");
    }

    #[test]
    fn embed_url_format() {
        let e = OllamaEmbedder::default_new();
        assert_eq!(e.embed_url(), "http://localhost:11434/api/embeddings");
    }

    #[test]
    fn name_is_ollama() {
        assert_eq!(OllamaEmbedder::default_new().name(), "ollama");
    }

    #[tokio::test]
    async fn happy_path_returns_embedding() {
        let v = fixed_vec(0.25);
        let v_clone = v.clone();
        let app = Router::new().route(
            "/api/embeddings",
            post(move |Json(body): Json<serde_json::Value>| {
                let v = v_clone.clone();
                async move {
                    assert_eq!(body["model"], "nomic-embed-text");
                    assert_eq!(body["prompt"], "hello world");
                    Json(serde_json::json!({ "embedding": v }))
                }
            }),
        );
        let url = start_mock(app).await;
        let e = OllamaEmbedder::new(url, String::new(), 0);
        let out = e.embed("hello world").await.unwrap();
        assert_eq!(out.len(), EMBEDDING_DIM);
        assert!((out[0] - 0.25).abs() < 1e-6);
    }

    #[tokio::test]
    async fn server_error_bubbles_up() {
        let app = Router::new().route(
            "/api/embeddings",
            post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "model crashed") }),
        );
        let url = start_mock(app).await;
        let e = OllamaEmbedder::new(url, String::new(), 0);
        let err = e.embed("hello").await.unwrap_err().to_string();
        assert!(err.contains("500"), "msg: {err}");
        assert!(err.contains("model crashed"), "msg: {err}");
    }

    #[tokio::test]
    async fn dim_mismatch_rejected() {
        let app = Router::new().route(
            "/api/embeddings",
            post(|| async {
                // Return a 3-dim vector — must fail validation.
                Json(serde_json::json!({ "embedding": [0.1, 0.2, 0.3] }))
            }),
        );
        let url = start_mock(app).await;
        let e = OllamaEmbedder::new(url, String::new(), 0);
        let err = e.embed("hi").await.unwrap_err().to_string();
        assert!(err.contains("3 dims"), "msg: {err}");
        assert!(err.contains("expected 768"), "msg: {err}");
    }

    #[tokio::test]
    async fn malformed_json_response_rejected() {
        let app = Router::new().route(
            "/api/embeddings",
            post(|| async { (StatusCode::OK, "not even json") }),
        );
        let url = start_mock(app).await;
        let e = OllamaEmbedder::new(url, String::new(), 0);
        let err = e.embed("hi").await.unwrap_err().to_string();
        assert!(err.contains("parse failed"), "msg: {err}");
    }

    #[tokio::test]
    async fn connection_refused_is_descriptive() {
        // Port 1 is effectively guaranteed refused on any reasonable host.
        let e = OllamaEmbedder::new("http://127.0.0.1:1".into(), String::new(), 500);
        let err = e.embed("hi").await.unwrap_err().to_string();
        assert!(
            err.contains("is Ollama running"),
            "should mention Ollama: {err}"
        );
    }
}
