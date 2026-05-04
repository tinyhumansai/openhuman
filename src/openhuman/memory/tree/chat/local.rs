//! Local Ollama chat provider — the legacy `llm_backend = "local"` path.
//!
//! Speaks Ollama's `/api/chat` with `format: "json"` and
//! `temperature: 0.0`. Mirrors what the per-extractor/summariser HTTP client
//! used to do, but behind the [`super::ChatProvider`] trait so the same
//! call site can be cloud-routed instead.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{ChatPrompt, ChatProvider};

/// Ollama-direct chat provider.
pub struct OllamaChatProvider {
    endpoint: String,
    model: String,
    http: Client,
    /// Cached display name `"local:ollama:<model>"` for logs.
    display: String,
}

impl OllamaChatProvider {
    /// Build the provider. `endpoint` and `model` may be `None` — when
    /// either is unset, [`ChatProvider::chat_for_json`] returns a clear
    /// error so the caller's soft-fallback path engages and the seal/admit
    /// pipeline keeps running.
    pub fn new(endpoint: Option<String>, model: Option<String>, timeout: Duration) -> Result<Self> {
        // No body-read timeout. Ollama is a local process — slow responses
        // mean the model is genuinely processing under CPU load (e.g.
        // gemma3:1b on CPU-only inference can take minutes per call), not
        // that the network broke. A body-read timeout here would cancel
        // mid-flight generation and force pointless retries against the
        // same slow model. `timeout` becomes the TCP connect timeout —
        // short enough to fail fast when Ollama is actually unreachable.
        let http = Client::builder()
            .connect_timeout(timeout)
            .build()
            .context("build ollama http client")?;
        let endpoint = endpoint.unwrap_or_default();
        let model = model.unwrap_or_default();
        let display = format!(
            "local:ollama:{}",
            if model.is_empty() { "<unset>" } else { &model }
        );
        Ok(Self {
            endpoint,
            model,
            http,
            display,
        })
    }
}

#[async_trait]
impl ChatProvider for OllamaChatProvider {
    fn name(&self) -> &str {
        &self.display
    }

    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String> {
        if self.endpoint.is_empty() || self.model.is_empty() {
            return Err(anyhow!(
                "[memory_tree::chat::local] Ollama endpoint or model not configured \
                 (endpoint_set={}, model_set={}); set memory_tree.llm_*_endpoint / \
                 _model in config or switch memory_tree.llm_backend to cloud",
                !self.endpoint.is_empty(),
                !self.model.is_empty()
            ));
        }
        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: prompt.system.clone(),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: prompt.user.clone(),
                },
            ],
            format: "json".to_string(),
            stream: false,
            options: OllamaOptions {
                temperature: prompt.temperature as f32,
            },
        };

        log::debug!(
            "[memory_tree::chat::local] POST {url} kind={} model={} sys_chars={} user_chars={}",
            prompt.kind,
            self.model,
            prompt.system.len(),
            prompt.user.len()
        );

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("ollama POST {url}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let snippet = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "ollama non-success status {status}: {}",
                truncate_for_log(&snippet, 200)
            ));
        }

        let envelope: OllamaChatResponse = resp
            .json()
            .await
            .context("decode ollama chat response envelope")?;
        log::debug!(
            "[memory_tree::chat::local] ollama response chars={} kind={}",
            envelope.message.content.len(),
            prompt.kind
        );
        Ok(envelope.message.content)
    }
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    format: String,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn errors_clearly_when_endpoint_missing() {
        let p = OllamaChatProvider::new(None, Some("m".into()), Duration::from_millis(50)).unwrap();
        let err = p
            .chat_for_json(&ChatPrompt {
                system: "s".into(),
                user: "u".into(),
                temperature: 0.0,
                kind: "test",
            })
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not configured"),
            "expected config error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn errors_when_model_missing() {
        let p = OllamaChatProvider::new(
            Some("http://localhost:11434".into()),
            None,
            Duration::from_millis(50),
        )
        .unwrap();
        let err = p
            .chat_for_json(&ChatPrompt {
                system: "s".into(),
                user: "u".into(),
                temperature: 0.0,
                kind: "test",
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not configured"));
    }

    #[tokio::test]
    async fn transport_failure_returns_err() {
        // Endpoint pointing at an unreachable port. The provider returns
        // Err — the consumer is responsible for soft-fallback.
        let p = OllamaChatProvider::new(
            Some("http://127.0.0.1:1".into()),
            Some("m".into()),
            Duration::from_millis(50),
        )
        .unwrap();
        let err = p
            .chat_for_json(&ChatPrompt {
                system: "s".into(),
                user: "u".into(),
                temperature: 0.0,
                kind: "test",
            })
            .await
            .unwrap_err();
        // Connection error chain — message contains "ollama POST" prefix.
        assert!(format!("{err}").contains("ollama POST"));
    }

    #[test]
    fn name_includes_model() {
        let p =
            OllamaChatProvider::new(None, Some("qwen2.5:0.5b".into()), Duration::from_millis(50))
                .unwrap();
        assert!(p.name().contains("qwen2.5:0.5b"));
        assert!(p.name().starts_with("local:ollama:"));
    }

    #[test]
    fn name_handles_unset_model() {
        let p = OllamaChatProvider::new(None, None, Duration::from_millis(50)).unwrap();
        assert!(p.name().contains("<unset>"));
    }

    #[test]
    fn truncate_for_log_short_unchanged() {
        assert_eq!(truncate_for_log("hi", 10), "hi");
    }

    #[test]
    fn truncate_for_log_long_appends_ellipsis() {
        let long = "x".repeat(500);
        let out = truncate_for_log(&long, 10);
        assert_eq!(out.chars().count(), 11);
        assert!(out.ends_with('…'));
    }
}
