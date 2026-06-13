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
    for (source, candidate) in [
        (
            "OPENHUMAN_LM_STUDIO_BASE_URL",
            std::env::var("OPENHUMAN_LM_STUDIO_BASE_URL").ok(),
        ),
        (
            "LM_STUDIO_BASE_URL",
            std::env::var("LM_STUDIO_BASE_URL").ok(),
        ),
        ("config.local_ai.base_url", local_ai.base_url.clone()),
    ] {
        let Some(raw) = candidate else {
            tracing::trace!(source, "[lm-studio] base URL candidate missing");
            continue;
        };
        tracing::trace!(
            source,
            raw = %redact_url_for_log(&raw),
            "[lm-studio] inspecting base URL candidate"
        );
        if let Some(normalized) = normalize_lm_studio_base_url(&raw) {
            tracing::debug!(
                source,
                base_url = %redact_url_for_log(&normalized),
                "[lm-studio] selected normalized base URL"
            );
            return normalized;
        }
        tracing::trace!(source, "[lm-studio] rejected blank base URL candidate");
    }

    tracing::debug!(
        base_url = %DEFAULT_LM_STUDIO_BASE_URL,
        "[lm-studio] using default base URL"
    );
    DEFAULT_LM_STUDIO_BASE_URL.to_string()
}

pub(crate) fn normalize_lm_studio_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    tracing::trace!(
        raw = %redact_url_for_log(raw),
        trimmed = %redact_url_for_log(trimmed),
        "[lm-studio] normalizing base URL"
    );
    if trimmed.is_empty() {
        tracing::trace!("[lm-studio] base URL normalization rejected blank input");
        return None;
    }

    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    tracing::trace!(
        with_scheme = %redact_url_for_log(&with_scheme),
        "[lm-studio] base URL scheme normalized"
    );

    let without_known_endpoint = with_scheme
        .trim_end_matches("/chat/completions")
        .trim_end_matches("/models")
        .trim_end_matches('/')
        .to_string();
    tracing::trace!(
        without_known_endpoint = %redact_url_for_log(&without_known_endpoint),
        "[lm-studio] base URL endpoint suffix normalized"
    );

    if without_known_endpoint.ends_with("/v1") {
        tracing::trace!(
            appended_v1 = false,
            base_url = %redact_url_for_log(&without_known_endpoint),
            "[lm-studio] base URL normalization complete"
        );
        Some(without_known_endpoint)
    } else {
        let normalized = format!("{without_known_endpoint}/v1");
        tracing::trace!(
            appended_v1 = true,
            base_url = %redact_url_for_log(&normalized),
            "[lm-studio] base URL normalization complete"
        );
        Some(normalized)
    }
}

pub(crate) fn apply_lm_studio_auth(
    request: reqwest::RequestBuilder,
    config: &Config,
) -> reqwest::RequestBuilder {
    match config.local_ai.api_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => {
            tracing::trace!(
                api_key_present = true,
                api_key_len = key.len(),
                "[lm-studio] auth applied"
            );
            request.bearer_auth(key)
        }
        _ => {
            tracing::trace!(api_key_present = false, "[lm-studio] auth skipped");
            request
        }
    }
}

fn redact_url_for_log(raw: &str) -> String {
    let trimmed = raw.trim();
    let parsed =
        url::Url::parse(trimmed).or_else(|_| url::Url::parse(&format!("http://{trimmed}")));
    let Ok(mut parsed) = parsed else {
        return trimmed.to_string();
    };
    if !parsed.username().is_empty() {
        let _ = parsed.set_username("redacted");
    }
    if parsed.password().is_some() {
        let _ = parsed.set_password(Some("redacted"));
    }
    parsed.to_string().trim_end_matches('/').to_string()
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
    /// Local reasoning models expose chain-of-thought as `reasoning_content`
    /// or `reasoning` depending on the runtime — accept both field names.
    #[serde(default, alias = "reasoning")]
    pub reasoning_content: Option<String>,
}

impl LmStudioChatResponseMessage {
    pub(crate) fn effective_content(&self) -> String {
        let content = self
            .content
            .as_deref()
            .map(crate::openhuman::inference::provider::compatible_parse::strip_think_tags)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        if !content.is_empty() {
            tracing::trace!(
                source = "content",
                output_chars = content.chars().count(),
                "[lm-studio] effective content selected"
            );
            return content;
        }

        let reasoning = self
            .reasoning_content
            .as_deref()
            .map(crate::openhuman::inference::provider::compatible_parse::strip_think_tags)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        if !reasoning.is_empty() {
            tracing::trace!(
                source = "reasoning_content",
                output_chars = reasoning.chars().count(),
                "[lm-studio] effective content selected"
            );
            return reasoning;
        }

        tracing::trace!(
            source = "none",
            output_chars = 0,
            "[lm-studio] effective content empty"
        );
        String::new()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
}

/// LM Studio **native** REST (`GET /api/v0/models`) model entry.
///
/// Unlike the OpenAI-compatible `/v1/models` (which returns only
/// `id`/`object`/`owned_by`), the native API reports the model's context
/// window — the value the agent harness must budget against to avoid an
/// `n_ctx` overflow when the user loaded the model with a small context
/// (issue #3550 / Sentry TAURI-RUST-6V0).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LmStudioNativeModel {
    pub id: String,
    /// Context window the model is *currently loaded* with — the runtime's
    /// hard limit. Authoritative for budgeting. (LM Studio also returns a
    /// `state` field, which we ignore — we prefer the loaded window whenever
    /// present regardless of load state.)
    #[serde(default)]
    pub loaded_context_length: Option<u64>,
    /// Model's declared maximum context. Fallback when not currently loaded.
    #[serde(default)]
    pub max_context_length: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LmStudioNativeModelsResponse {
    #[serde(default)]
    pub data: Vec<LmStudioNativeModel>,
}

/// Map a normalized `…/v1` base URL to the LM Studio native models endpoint
/// `…/api/v0/models` (a sibling of `/v1`, served at the host root).
pub(crate) fn lm_studio_native_models_url(v1_base_url: &str) -> String {
    let root = v1_base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/');
    format!("{root}/api/v0/models")
}

/// Resolve the context window LM Studio reports for `model_id` from a native
/// `/api/v0/models` payload: prefer the *loaded* context (the limit the
/// runtime actually enforces), else the model's declared maximum. Zero/absent
/// values are treated as unknown. Returns `None` when the model isn't present
/// or reports no usable window.
pub(crate) fn lm_studio_context_window_for(
    resp: &LmStudioNativeModelsResponse,
    model_id: &str,
) -> Option<u64> {
    resp.data.iter().find(|m| m.id == model_id).and_then(|m| {
        m.loaded_context_length
            .filter(|&v| v > 0)
            .or(m.max_context_length.filter(|&v| v > 0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_models_url_derived_from_v1_base() {
        assert_eq!(
            lm_studio_native_models_url("http://localhost:1234/v1"),
            "http://localhost:1234/api/v0/models"
        );
        // Trailing slash tolerated.
        assert_eq!(
            lm_studio_native_models_url("http://127.0.0.1:1234/v1/"),
            "http://127.0.0.1:1234/api/v0/models"
        );
        // Remote host with path prefix.
        assert_eq!(
            lm_studio_native_models_url("https://lm.example.com/lmstudio/v1"),
            "https://lm.example.com/lmstudio/api/v0/models"
        );
    }

    #[test]
    fn context_window_prefers_loaded_then_max() {
        let resp: LmStudioNativeModelsResponse = serde_json::from_str(
            r#"{"data":[
                {"id":"qwen2.5-7b","state":"loaded","loaded_context_length":4096,"max_context_length":32768},
                {"id":"phi-4","state":"not-loaded","max_context_length":16384}
            ]}"#,
        )
        .unwrap();
        // Loaded model → the runtime-enforced loaded window, NOT the trained max.
        assert_eq!(
            lm_studio_context_window_for(&resp, "qwen2.5-7b"),
            Some(4096)
        );
        // Not-loaded model → declared max as fallback.
        assert_eq!(lm_studio_context_window_for(&resp, "phi-4"), Some(16384));
        // Unknown model id → None (caller falls back to profile default).
        assert_eq!(lm_studio_context_window_for(&resp, "missing"), None);
    }

    #[test]
    fn context_window_treats_zero_and_absent_as_unknown() {
        let resp: LmStudioNativeModelsResponse = serde_json::from_str(
            r#"{"data":[
                {"id":"zeroed","loaded_context_length":0,"max_context_length":0},
                {"id":"bare"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(lm_studio_context_window_for(&resp, "zeroed"), None);
        assert_eq!(lm_studio_context_window_for(&resp, "bare"), None);
    }

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

    #[test]
    fn effective_content_falls_back_to_reasoning_content() {
        let msg = LmStudioChatResponseMessage {
            content: Some("".into()),
            reasoning_content: Some("thinking text".into()),
        };
        assert_eq!(msg.effective_content(), "thinking text");
    }

    #[test]
    fn effective_content_strips_think_tags() {
        let msg = LmStudioChatResponseMessage {
            content: Some("<think>hidden</think>Visible reply".into()),
            reasoning_content: None,
        };
        assert_eq!(msg.effective_content(), "Visible reply");
    }

    #[test]
    fn reasoning_content_accepts_reasoning_alias() {
        // Local runtimes that name the field `reasoning` must still be captured
        // (issue #3094) so reasoning round-trips like the canonical field.
        let msg: LmStudioChatResponseMessage =
            serde_json::from_str(r#"{"content":null,"reasoning":"local cot"}"#).unwrap();
        assert_eq!(msg.reasoning_content.as_deref(), Some("local cot"));
    }
}
