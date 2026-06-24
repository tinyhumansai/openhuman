//! Generic OpenAI-compatible provider.
//! Most LLM APIs follow the same `/v1/chat/completions` format.
//! This module provides a single implementation that works for all of them.

#[path = "compatible_dump.rs"]
mod compatible_dump;
#[path = "compatible_helpers.rs"]
mod compatible_helpers;
#[path = "compatible_parse.rs"]
mod compatible_parse;
#[path = "compatible_provider_impl.rs"]
mod compatible_provider_impl;
#[path = "compatible_repeat.rs"]
mod compatible_repeat;
#[path = "compatible_request.rs"]
mod compatible_request;
#[path = "compatible_stream.rs"]
mod compatible_stream;
#[path = "compatible_stream_native.rs"]
mod compatible_stream_native;
#[path = "compatible_timeout.rs"]
mod compatible_timeout;
#[path = "compatible_types.rs"]
mod compatible_types;

#[cfg(test)]
pub(crate) use super::traits::{ChatMessage, ConversationMessage, Provider};
#[cfg(test)]
pub(crate) use compatible_parse::normalize_function_arguments;
#[cfg(test)]
pub(crate) use compatible_parse::{
    build_responses_prompt, extract_responses_text, parse_chat_response_body,
    parse_provider_tool_call_from_value, parse_responses_response_body, parse_sse_line,
    strip_think_tags,
};
#[cfg(test)]
pub(crate) use compatible_repeat::{StreamRepeatDetector, STREAM_REPEAT_THRESHOLD};
#[cfg(test)]
pub(crate) use compatible_types::StreamChunkResponse;
#[cfg(test)]
pub(crate) use compatible_types::{
    ApiChatRequest, ApiChatResponse, Choice, Function, Message, MessageContent, NativeChatRequest,
    NativeMessage, ResponseMessage, ResponsesResponse, ToolCall,
};

/// A provider that speaks the OpenAI-compatible chat completions API.
/// Used by: Venice, Vercel AI Gateway, Cloudflare AI Gateway, Moonshot,
/// Synthetic, `OpenCode` Zen, `Z.AI`, `GLM`, `MiniMax`, Bedrock, Qianfan, Groq, Mistral, `xAI`, etc.
pub struct OpenAiCompatibleProvider {
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) credential: Option<String>,
    pub(crate) auth_header: AuthStyle,
    /// When false, do not fall back to /v1/responses on chat completions 404.
    /// GLM/Zhipu does not support the responses API.
    supports_responses_fallback: bool,
    /// When true, call the Responses API directly instead of first trying
    /// chat completions. Required for ChatGPT-account Codex OAuth.
    responses_api_primary: bool,
    user_agent: Option<String>,
    extra_headers: Vec<(String, String)>,
    extra_query_params: Vec<(String, String)>,
    /// When true, collect all `system` messages and prepend their content
    /// to the first `user` message, then drop the system messages.
    /// Required for providers that reject `role: system` (e.g. MiniMax).
    merge_system_into_user: bool,
    /// When true, forward the OpenHuman backend extension `thread_id`
    /// (read from `thread_context::current_thread_id`) on outbound
    /// chat completions bodies. Off by default — only the
    /// `OpenHumanBackendProvider` opts in, so third-party
    /// OpenAI-compatible endpoints (Venice, Moonshot, Groq, GLM, …)
    /// never see an unrecognized field that could trip strict input
    /// validation.
    emit_openhuman_thread_id: bool,
    /// Shell-style glob patterns (`*` only) for model IDs that MUST NOT
    /// receive a `temperature` field. Matches are done by
    /// `temperature::glob_match`. Defaults to empty (all models support
    /// temperature); populated by the factory when the config has entries.
    pub(crate) temperature_unsupported_models: Vec<String>,
    /// Per-workload temperature override. When `Some`, replaces the
    /// caller-supplied `temperature` for every chat call on this provider
    /// instance — set by the factory when the workload's provider string
    /// carries an `@<temp>` suffix (e.g. `"openai:gpt-4o@0.2"`). The
    /// `temperature_unsupported_models` glob filter still applies after.
    pub(crate) temperature_override: Option<f64>,
    /// Value reported by `capabilities().native_tool_calling`. Defaults to
    /// `true` because most OpenAI-compatible providers implement the
    /// `tools` parameter correctly. The factory flips this to `false` for
    /// Ollama, whose OpenAI-compat endpoint returns HTTP 400 on `tools`
    /// for many models.
    native_tool_calling: bool,
    /// Value reported by `capabilities().vision`. Defaults to `true` because
    /// OpenAI-compatible cloud endpoints accept the `image_url` message
    /// content shape. Local compatibility shims opt out unless they can prove
    /// the configured model supports vision.
    vision: bool,
    /// Ollama-specific `options.num_ctx` override. When set, every request
    /// to this provider includes `"options": {"num_ctx": <value>}` in the
    /// body so Ollama allocates the requested KV-cache size.
    pub(crate) ollama_num_ctx: Option<u32>,
    /// The local provider kind, if this is a local provider.
    /// Used for profile-aware context window resolution and diagnostics.
    pub(crate) local_provider_kind:
        Option<crate::openhuman::inference::local::profile::LocalProviderKind>,
}

/// How the provider expects the API key to be sent.
#[derive(Debug, Clone)]
pub enum AuthStyle {
    /// No authentication header.
    None,
    /// `Authorization: Bearer <key>`
    Bearer,
    /// `x-api-key: <key>` (used by some Chinese providers)
    XApiKey,
    /// Anthropic-specific: `x-api-key: <key>` + `anthropic-version: 2023-06-01`
    Anthropic,
    /// Custom header name
    Custom(String),
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
    ) -> Self {
        Self::new_with_options(name, base_url, credential, auth_style, true, None, false)
    }

    /// Same as `new` but skips the /v1/responses fallback on 404.
    /// Use for providers (e.g. GLM) that only support chat completions.
    pub fn new_no_responses_fallback(
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
    ) -> Self {
        Self::new_with_options(name, base_url, credential, auth_style, false, None, false)
    }

    /// Create a provider with a custom User-Agent header.
    ///
    /// Some providers (for example Kimi Code) require a specific User-Agent
    /// for request routing and policy enforcement.
    pub fn new_with_user_agent(
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
        user_agent: &str,
    ) -> Self {
        Self::new_with_options(
            name,
            base_url,
            credential,
            auth_style,
            true,
            Some(user_agent),
            false,
        )
    }

    /// For providers that do not support `role: system` (e.g. MiniMax).
    /// System prompt content is prepended to the first user message instead.
    pub fn new_merge_system_into_user(
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
    ) -> Self {
        Self::new_with_options(name, base_url, credential, auth_style, false, None, true)
    }

    /// Opt this provider into emitting the OpenHuman backend extension
    /// `thread_id` on outbound chat completions bodies. Only the
    /// `OpenHumanBackendProvider` should call this — third-party
    /// OpenAI-compatible providers must leave it off so they don't
    /// receive an unknown field.
    pub fn with_openhuman_thread_id(mut self) -> Self {
        self.emit_openhuman_thread_id = true;
        self
    }

    fn new_with_options(
        name: &str,
        base_url: &str,
        credential: Option<&str>,
        auth_style: AuthStyle,
        supports_responses_fallback: bool,
        user_agent: Option<&str>,
        merge_system_into_user: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            credential: credential.map(ToString::to_string),
            auth_header: auth_style,
            supports_responses_fallback,
            responses_api_primary: false,
            user_agent: user_agent.map(ToString::to_string),
            extra_headers: Vec::new(),
            extra_query_params: Vec::new(),
            merge_system_into_user,
            emit_openhuman_thread_id: false,
            temperature_unsupported_models: Vec::new(),
            temperature_override: None,
            native_tool_calling: true,
            vision: true,
            ollama_num_ctx: None,
            local_provider_kind: None,
        }
    }

    /// Toggle whether this provider advertises native (OpenAI-style) tool
    /// calling to the agent harness. The default is `true`; set to `false`
    /// for providers whose `/v1/chat/completions` endpoint rejects the
    /// `tools` parameter.
    pub fn with_native_tool_calling(mut self, enabled: bool) -> Self {
        self.native_tool_calling = enabled;
        self
    }

    /// Toggle whether this provider advertises OpenAI-compatible vision input.
    /// Cloud providers default to enabled; local OpenAI-compatible shims use
    /// this to stay fail-closed for text-only local models.
    pub fn with_vision(mut self, enabled: bool) -> Self {
        self.vision = enabled;
        self
    }

    /// Set the list of model glob patterns for which temperature must be
    /// omitted from request bodies.
    pub fn with_temperature_unsupported_models(mut self, patterns: Vec<String>) -> Self {
        self.temperature_unsupported_models = patterns;
        self
    }

    /// Pin a per-workload temperature, overriding whatever the caller passes.
    pub fn with_temperature_override(mut self, temperature: Option<f64>) -> Self {
        self.temperature_override = temperature;
        self
    }

    /// Set the Ollama `options.num_ctx` override.
    pub fn with_ollama_num_ctx(mut self, num_ctx: Option<u32>) -> Self {
        self.ollama_num_ctx = num_ctx;
        self
    }

    /// Tag this provider with its local provider kind for profile-aware
    /// context window resolution and diagnostics.
    pub fn with_local_provider_kind(
        mut self,
        kind: crate::openhuman::inference::local::profile::LocalProviderKind,
    ) -> Self {
        self.local_provider_kind = Some(kind);
        self
    }

    pub fn with_extra_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        if !name.trim().is_empty() && !value.trim().is_empty() {
            self.extra_headers
                .push((name.trim().to_string(), value.trim().to_string()));
        }
        self
    }

    pub fn with_user_agent(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.trim().is_empty() {
            self.user_agent = Some(value.trim().to_string());
        }
        self
    }

    pub fn with_responses_api_primary(mut self) -> Self {
        self.responses_api_primary = true;
        self
    }

    pub fn with_extra_query_param(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let value = value.into();
        if !name.trim().is_empty() && !value.trim().is_empty() {
            self.extra_query_params
                .push((name.trim().to_string(), value.trim().to_string()));
        }
        self
    }
}

/// Prompt-cache behaviour for an OpenAI-compatible provider, keyed on its
/// configured slug (#3939).
///
/// Conservative by default: only providers we have verified to both (a) cache
/// identical request prefixes server-side and (b) report cached input tokens
/// via the OpenAI `prompt_tokens_details.cached_tokens` shape that
/// [`OpenAiCompatibleProvider`]'s usage extractor already normalises are opted
/// in. Unknown / custom slugs (including local LM Studio endpoints, which do no
/// server-side billing-grade caching) get the all-`false` default, so we never
/// advertise caching a custom endpoint may not do. `explicit_cache_control`
/// stays `false` for every OpenAI-compatible provider — the chat-completions
/// API has no cache-control field — and `cache_key_grouping` is reserved for
/// the OpenHuman backend's `thread_id` extension, declared on
/// `OpenHumanBackendProvider` directly.
pub(crate) fn prompt_cache_for_compatible_slug(
    slug: &str,
) -> super::traits::PromptCacheCapabilities {
    // Compare on the leading slug segment so a user-renamed `openai-eu` or a
    // `openai:gpt-5.1` style slug still resolves to the `openai` family.
    let normalized = slug.trim().to_ascii_lowercase();
    let family = normalized
        .split(|c| c == ':' || c == '/' || c == '-')
        .next()
        .unwrap_or("")
        .trim();

    // Verified OpenAI-style implicit prefix cache + cached-token usage reporting.
    let openai_style_cache = matches!(family, "openai" | "openrouter" | "gmi");

    let caps = super::traits::PromptCacheCapabilities {
        automatic_prefix_cache: openai_style_cache,
        explicit_cache_control: false,
        usage_reports_cached_input: openai_style_cache,
        cache_key_grouping: false,
    };

    tracing::debug!(
        domain = "llm_provider",
        operation = "prompt_cache_capability",
        provider = %family,
        automatic_prefix_cache = caps.automatic_prefix_cache,
        explicit_cache_control = caps.explicit_cache_control,
        usage_reports_cached_input = caps.usage_reports_cached_input,
        cache_key_grouping = caps.cache_key_grouping,
        "[llm_provider] prompt-cache capability selected for compatible provider"
    );

    caps
}

#[cfg(test)]
#[path = "compatible_tests.rs"]
mod tests;
