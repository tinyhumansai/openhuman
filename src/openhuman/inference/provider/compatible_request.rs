use crate::openhuman::inference::provider::traits::ChatMessage;
use reqwest::{
    header::{HeaderMap, HeaderValue, USER_AGENT},
    Client,
};

use crate::openhuman::inference::provider::{temperature, thread_context};

use super::{AuthStyle, OpenAiCompatibleProvider};

const OPENROUTER_REFERER: &str = "https://openhuman.ai";
const OPENROUTER_TITLE: &str = "OpenHuman";

impl OpenAiCompatibleProvider {
    /// Build the Ollama-specific `options` block for the request body.
    /// Returns `None` when no `num_ctx` override is configured.
    pub(super) fn build_ollama_options(&self) -> Option<super::compatible_types::OllamaOptions> {
        self.ollama_num_ctx
            .map(|num_ctx| super::compatible_types::OllamaOptions {
                num_ctx: Some(num_ctx),
            })
    }

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

    /// Resolve the `frequency_penalty` to send on streaming chat requests.
    ///
    /// Returns `None` — omitting the field entirely — for endpoints whose
    /// OpenAI-compatible surface rejects the parameter with an HTTP 400
    /// (see [`endpoint_rejects_frequency_penalty`]). Google's Gemini shim
    /// (`generativelanguage.googleapis.com/v1beta/openai`) is the known case:
    /// it 400s on the unknown field, which previously forced every streaming
    /// call into a wasted reject→retry round-trip and one Sentry report
    /// (TAURI-RUST-4PJ). Omitting it up front removes the failing request at
    /// the source. Every other provider keeps the configured penalty.
    pub(super) fn effective_frequency_penalty(&self) -> Option<f64> {
        if endpoint_rejects_frequency_penalty(&self.base_url) {
            tracing::debug!(
                "[provider:{}] endpoint rejects frequency_penalty — omitting it",
                self.name
            );
            None
        } else {
            Some(super::compatible_repeat::CHAT_FREQUENCY_PENALTY)
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
                .timeout(super::compatible_timeout::request_timeout())
                .connect_timeout(super::compatible_timeout::connect_timeout())
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
            .timeout(super::compatible_timeout::request_timeout())
            .connect_timeout(super::compatible_timeout::connect_timeout());
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
        let url = self.apply_extra_query_params(url);
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
        let url = if self.path_ends_with("/responses") {
            self.base_url.clone()
        } else {
            let normalized_base = self.base_url.trim_end_matches('/');

            // If chat endpoint is explicitly configured, derive sibling responses endpoint.
            if let Some(prefix) = normalized_base.strip_suffix("/chat/completions") {
                format!("{prefix}/responses")
            } else if self.has_explicit_api_path() {
                // If an explicit API path already exists (e.g. /v1, /openai, /api/coding/v3),
                // append responses directly to avoid duplicate /v1 segments.
                format!("{normalized_base}/responses")
            } else {
                format!("{normalized_base}/v1/responses")
            }
        };

        self.apply_extra_query_params(url)
    }

    pub(super) fn apply_extra_query_params(&self, url: String) -> String {
        if self.extra_query_params.is_empty() {
            return url;
        }

        if let Ok(mut parsed) = reqwest::Url::parse(&url) {
            {
                let mut pairs = parsed.query_pairs_mut();
                for (name, value) in &self.extra_query_params {
                    pairs.append_pair(name, value);
                }
            }
            return parsed.to_string();
        }

        let mut output = url;
        for (index, (name, value)) in self.extra_query_params.iter().enumerate() {
            let separator = if output.contains('?') || index > 0 {
                '&'
            } else {
                '?'
            };
            output.push(separator);
            output.push_str(name);
            output.push('=');
            output.push_str(value);
        }
        output
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
        let req = match (&self.auth_header, credential) {
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
        };
        self.apply_openrouter_attribution_headers(self.apply_extra_headers(req))
    }

    pub(super) fn apply_extra_headers(
        &self,
        mut req: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Some(user_agent) = self.user_agent.as_deref() {
            req = req.header(USER_AGENT, user_agent);
        }
        for (name, value) in &self.extra_headers {
            req = req.header(name.as_str(), value.as_str());
        }
        req
    }

    pub(super) fn is_openrouter_endpoint(&self) -> bool {
        if self.name.eq_ignore_ascii_case("openrouter") {
            return true;
        }

        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host == "openrouter.ai" || host.ends_with(".openrouter.ai"))
    }

    pub(super) fn apply_openrouter_attribution_headers(
        &self,
        req: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Some((referer, title)) = self.openrouter_attribution_headers() {
            req.header("HTTP-Referer", referer)
                .header("X-OpenRouter-Title", title)
        } else {
            req
        }
    }

    pub(super) fn openrouter_attribution_headers(&self) -> Option<(&'static str, &'static str)> {
        self.is_openrouter_endpoint()
            .then_some((OPENROUTER_REFERER, OPENROUTER_TITLE))
    }
}

/// Endpoint hosts whose OpenAI-compatible surface rejects the
/// `frequency_penalty` sampling field with an HTTP 400. Matched against the
/// request host (exact or as a registrable-domain suffix), so a BYOK provider
/// pointed at the same upstream is covered too. Single source of truth — add a
/// host here if another strict endpoint surfaces the same rejection.
const FREQUENCY_PENALTY_UNSUPPORTED_HOSTS: &[&str] = &["generativelanguage.googleapis.com"];

/// Whether `base_url`'s host is known to reject `frequency_penalty`.
///
/// Google's Gemini OpenAI-compat shim
/// (`https://generativelanguage.googleapis.com/v1beta/openai`) 400s on the
/// unknown field (`Unknown name "frequency_penalty": Cannot find field`),
/// which previously forced every streaming call into a wasted reject→retry
/// and one Sentry report per first attempt (TAURI-RUST-4PJ). Detecting it by
/// host — rather than provider slug — also covers BYOK providers configured
/// against the same endpoint.
pub(super) fn endpoint_rejects_frequency_penalty(base_url: &str) -> bool {
    let host = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
    let Some(host) = host else {
        return false;
    };
    FREQUENCY_PENALTY_UNSUPPORTED_HOSTS
        .iter()
        .any(|known| host == *known || host.ends_with(&format!(".{known}")))
}
