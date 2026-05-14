//! LM Studio OpenAI-compatible HTTP types and helpers.
//!
//! LM Studio exposes an OpenAI-compatible API under `http://localhost:1234/v1`
//! by default. This module keeps the wire contract separate from the Ollama
//! native API structs so the two providers can evolve independently.

use crate::openhuman::config::{Config, LocalAiConfig};
use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_LM_STUDIO_BASE_URL: &str = "http://localhost:1234/v1";

pub(crate) fn lm_studio_base_url(config: &Config) -> String {
    lm_studio_base_url_from_local_ai(&config.local_ai)
}

pub(crate) fn lm_studio_base_url_from_local_ai(local_ai: &LocalAiConfig) -> String {
    for raw in [
        std::env::var("OPENHUMAN_LM_STUDIO_BASE_URL").ok(),
        std::env::var("LM_STUDIO_BASE_URL").ok(),
        local_ai.base_url.clone(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(normalized) = normalize_lm_studio_base_url(&raw) {
            return normalized;
        }
    }

    DEFAULT_LM_STUDIO_BASE_URL.to_string()
}

pub(crate) fn normalize_lm_studio_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };

    let without_known_endpoint = with_scheme
        .trim_end_matches("/chat/completions")
        .trim_end_matches("/models")
        .trim_end_matches('/')
        .to_string();

    if without_known_endpoint.ends_with("/v1") {
        Some(without_known_endpoint)
    } else {
        Some(format!("{without_known_endpoint}/v1"))
    }
}

pub(crate) fn apply_lm_studio_auth(
    request: reqwest::RequestBuilder,
    config: &Config,
) -> reqwest::RequestBuilder {
    match config.local_ai.api_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => request.bearer_auth(key),
        _ => request,
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioModelsResponse {
    #[serde(default)]
    pub data: Vec<LmStudioModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct LmStudioModel {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub owned_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LmStudioChatCompletionRequest {
    pub model: String,
    pub messages: Vec<LmStudioChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LmStudioChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioChatCompletionResponse {
    #[serde(default)]
    pub choices: Vec<LmStudioChatChoice>,
    #[serde(default)]
    pub usage: Option<LmStudioUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioChatChoice {
    pub message: LmStudioChatResponseMessage,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioChatResponseMessage {
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lm_studio_base_url_defaults_scheme_and_v1() {
        assert_eq!(
            normalize_lm_studio_base_url("localhost:1234").as_deref(),
            Some("http://localhost:1234/v1")
        );
    }

    #[test]
    fn normalize_lm_studio_base_url_preserves_existing_v1() {
        assert_eq!(
            normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/").as_deref(),
            Some("http://127.0.0.1:1234/v1")
        );
    }

    #[test]
    fn normalize_lm_studio_base_url_strips_known_endpoint_suffix() {
        assert_eq!(
            normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/chat/completions").as_deref(),
            Some("http://127.0.0.1:1234/v1")
        );
        assert_eq!(
            normalize_lm_studio_base_url("http://127.0.0.1:1234/v1/models").as_deref(),
            Some("http://127.0.0.1:1234/v1")
        );
    }
}
