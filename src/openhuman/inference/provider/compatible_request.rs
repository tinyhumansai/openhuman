use crate::openhuman::inference::provider::traits::ChatMessage;
use reqwest::{
    header::{HeaderMap, HeaderValue, USER_AGENT},
    Client,
};

use crate::openhuman::inference::provider::{temperature, thread_context};

use super::{AuthStyle, OpenAiCompatibleProvider};

impl OpenAiCompatibleProvider {
    /// Resolve the effective temperature for `model`. Returns `None` when the
    /// model matches a pattern in `temperature_unsupported_models` (causing the
    /// field to be omitted from the serialised request). Otherwise yields the
    /// per-workload override if one was configured, else the caller's value.
    pub(super) fn effective_temperature(&self, model: &str, temperature: f64) -> Option<f64> {
        if self
            .temperature_unsupported_models
            .iter()
            .any(|pat| temperature::glob_match(pat, model))
        {
            tracing::debug!(
            "[provider:{}] model='{}' matched temperature_unsupported_models — omitting temperature",
            self.name,
            model
        );
            None
        } else {
            Some(self.temperature_override.unwrap_or(temperature))
        }
    }

    /// Read the ambient `thread_id` only when this provider has been
    /// opted in via [`with_openhuman_thread_id`]. Returns `None` for
    /// every third-party provider so the field is omitted by
    /// `skip_serializing_if`.
    pub(super) fn outbound_thread_id(&self) -> Option<String> {
        if self.emit_openhuman_thread_id {
            thread_context::current_thread_id()
        } else {
            None
        }
    }

    /// Collect all `system` role messages, concatenate their content,
    /// and prepend to the first `user` message. Drop all system messages.
    /// Used for providers (e.g. MiniMax) that reject `role: system`.
    pub(super) fn flatten_system_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let system_content: String = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        if system_content.is_empty() {
            return messages.to_vec();
        }

        let mut result: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role != "system")
            .cloned()
            .collect();

        if let Some(first_user) = result.iter_mut().find(|m| m.role == "user") {
            first_user.content = format!("{system_content}\n\n{}", first_user.content);
        } else {
            // No user message found: insert a synthetic user message with system content
            result.insert(0, ChatMessage::user(&system_content));
        }

        result
    }

    pub(super) fn http_client(&self) -> Client {
        if let Some(ua) = self.user_agent.as_deref() {
            let mut headers = HeaderMap::new();
            if let Ok(value) = HeaderValue::from_str(ua) {
                headers.insert(USER_AGENT, value);
            }

            // Platform-appropriate TLS backend — see [`crate::openhuman::tls`].
            let builder = crate::openhuman::tls::tls_client_builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(10))
                .default_headers(headers);
            let builder = crate::openhuman::config::apply_runtime_proxy_to_builder(
                builder,
                "provider.compatible",
            );

            return builder.build().unwrap_or_else(|error| {
                tracing::warn!("Failed to build proxied timeout client with user-agent: {error}");
                crate::openhuman::tls::tls_client_builder()
                    .build()
                    .unwrap_or_default()
            });
        }

        // Platform-appropriate TLS backend — see [`crate::openhuman::tls`].
        let builder = crate::openhuman::tls::tls_client_builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10));
        let builder = crate::openhuman::config::apply_runtime_proxy_to_builder(
            builder,
            "provider.compatible",
        );
        builder.build().unwrap_or_else(|error| {
            tracing::warn!("Failed to build proxied timeout client: {error}");
            crate::openhuman::tls::tls_client_builder()
                .build()
                .unwrap_or_default()
        })
    }

    /// Build the full URL for chat completions, detecting if base_url already includes the path.
    /// This allows custom providers with non-standard endpoints (e.g., VolcEngine ARK uses
    /// `/api/coding/v3/chat/completions` instead of `/v1/chat/completions`).
    pub(super) fn chat_completions_url(&self) -> String {
        let has_full_endpoint = reqwest::Url::parse(&self.base_url)
            .map(|url| {
                url.path()
                    .trim_end_matches('/')
                    .ends_with("/chat/completions")
            })
            .unwrap_or_else(|_| {
                self.base_url
                    .trim_end_matches('/')
                    .ends_with("/chat/completions")
            });

        let url = if has_full_endpoint {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        };
        log::info!(
            "[provider:{}] outbound chat/completions -> {}",
            self.name,
            url
        );
        url
    }

    pub(super) fn path_ends_with(&self, suffix: &str) -> bool {
        if let Ok(url) = reqwest::Url::parse(&self.base_url) {
            return url.path().trim_end_matches('/').ends_with(suffix);
        }

        self.base_url.trim_end_matches('/').ends_with(suffix)
    }

    pub(super) fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    /// Build the full URL for responses API, detecting if base_url already includes the path.
    pub(super) fn responses_url(&self) -> String {
        if self.path_ends_with("/responses") {
            return self.base_url.clone();
        }

        let normalized_base = self.base_url.trim_end_matches('/');

        // If chat endpoint is explicitly configured, derive sibling responses endpoint.
        if let Some(prefix) = normalized_base.strip_suffix("/chat/completions") {
            return format!("{prefix}/responses");
        }

        // If an explicit API path already exists (e.g. /v1, /openai, /api/coding/v3),
        // append responses directly to avoid duplicate /v1 segments.
        if self.has_explicit_api_path() {
            format!("{normalized_base}/responses")
        } else {
            format!("{normalized_base}/v1/responses")
        }
    }

    pub(super) fn tool_specs_to_openai_format(
        tools: &[crate::openhuman::tools::ToolSpec],
    ) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters
                    }
                })
            })
            .collect()
    }

    pub(super) fn credential_for_request(&self) -> anyhow::Result<Option<&str>> {
        if matches!(&self.auth_header, AuthStyle::None) {
            return Ok(None);
        }

        self.credential
            .as_deref()
            .map(str::trim)
            .filter(|credential| !credential.is_empty())
            .map(Some)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} API key not set. Configure via the web UI or set the appropriate env var.",
                    self.name
                )
            })
    }

    pub(super) fn apply_auth_header(
        &self,
        req: reqwest::RequestBuilder,
        credential: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match (&self.auth_header, credential) {
            (AuthStyle::None, _) => req,
            (_, None) => req,
            (AuthStyle::Bearer, Some(credential)) => {
                req.header("Authorization", format!("Bearer {credential}"))
            }
            (AuthStyle::XApiKey, Some(credential)) => req.header("x-api-key", credential),
            (AuthStyle::Anthropic, Some(credential)) => req
                .header("x-api-key", credential)
                .header("anthropic-version", "2023-06-01"),
            (AuthStyle::Custom(header), Some(credential)) => req.header(header, credential),
        }
    }
}
