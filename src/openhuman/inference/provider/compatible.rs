//! Generic OpenAI-compatible provider.
//! Most LLM APIs follow the same `/v1/chat/completions` format.
//! This module provides a single implementation that works for all of them.

#[path = "compatible_dump.rs"]
mod compatible_dump;
#[path = "compatible_parse.rs"]
mod compatible_parse;
#[path = "compatible_request.rs"]
mod compatible_request;
#[path = "compatible_stream.rs"]
mod compatible_stream;
#[path = "compatible_types.rs"]
mod compatible_types;

#[cfg(test)]
pub(crate) use compatible_parse::{
    parse_provider_tool_call_from_value, parse_sse_line, strip_think_tags,
};
#[cfg(test)]
pub(crate) use compatible_types::ResponsesResponse;

use crate::openhuman::inference::provider::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, StreamChunk, StreamError, StreamOptions, StreamResult, ToolCall as ProviderToolCall,
    UsageInfo as ProviderUsageInfo,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};

use compatible_dump::{dump_prompt_if_enabled, dump_response_if_enabled, reserve_dump_seq};
use compatible_parse::{
    aggregate_responses_sse_body, build_responses_prompt, extract_responses_text,
    normalize_function_arguments, parse_chat_response_body, parse_responses_response_body,
    parse_tool_calls_from_content_json,
};
use compatible_stream::sse_bytes_to_chunks;
use compatible_types::{
    ApiChatRequest, ApiChatResponse, ApiUsage, Choice, Function, Message, MessageContent,
    NativeChatRequest, NativeMessage, OpenAiStreamOptions, OpenHumanMeta, ResponseMessage,
    ResponsesRequest, StreamChunkResponse, StreamingToolCall, ToolCall,
};

/// `frequency_penalty` applied to streaming chat-completions requests.
///
/// Autoregressive models have a self-reinforcing bias toward repeating spans
/// already in their context; with no penalty a momentary repeat can spiral into
/// the same line emitted until the output-token cap (degenerate decoding). A
/// small positive penalty damps that loop without harming coherence. Carried on
/// the streaming path (where those loops occur — long autonomous turns) and
/// retried without it if a strict provider rejects it; the buffered
/// non-streaming fallback omits it for maximum compatibility. Skipped in
/// serialisation when `None` so providers that don't accept the field are
/// unaffected.
const CHAT_FREQUENCY_PENALTY: f64 = 0.3;

/// Consecutive identical substantial lines that trip the in-generation repeat
/// cutoff. Autoregressive models can latch onto a line and emit it verbatim
/// until the token cap (observed: 234× the same sentence in one response).
/// `frequency_penalty` / stronger model tiers only lower the odds — they don't
/// prevent it — so this is the deterministic, model-agnostic stop. Set well
/// above any legitimate repetition.
const STREAM_REPEAT_THRESHOLD: u32 = 6;
/// Minimum trimmed length for a line to count toward [`STREAM_REPEAT_THRESHOLD`].
/// Keeps short, legitimately-repeated lines (`}`, blank-ish code) from tripping
/// it; degenerate spirals are long sentences well over this.
const MIN_REPEAT_LINE_CHARS: usize = 16;

/// Detects in-generation repetition degeneration on the streaming path so the
/// reader can abort the stream and truncate the blob. Trips after
/// [`STREAM_REPEAT_THRESHOLD`] consecutive identical substantial lines; blank
/// separator lines are ignored, so `"sentence\n\nsentence\n\n…"` still trips.
#[derive(Default)]
struct StreamRepeatDetector {
    current_line: String,
    last_line: Option<String>,
    consecutive: u32,
}

impl StreamRepeatDetector {
    fn new() -> Self {
        Self::default()
    }

    /// Feed one streamed text delta. Returns `true` once the same substantial
    /// line has repeated [`STREAM_REPEAT_THRESHOLD`] times back-to-back.
    fn observe(&mut self, delta: &str) -> bool {
        for ch in delta.chars() {
            if ch == '\n' {
                if self.finalize_line() {
                    return true;
                }
            } else {
                self.current_line.push(ch);
            }
        }
        false
    }

    fn finalize_line(&mut self) -> bool {
        let line = self.current_line.trim().to_string();
        self.current_line.clear();
        if line.is_empty() {
            // Blank separator between repeats — ignore, don't reset the run.
            return false;
        }
        if line.chars().count() < MIN_REPEAT_LINE_CHARS {
            // Short line — not a degenerate-sentence repeat; reset the run.
            self.last_line = Some(line);
            self.consecutive = 1;
            return false;
        }
        if self.last_line.as_deref() == Some(line.as_str()) {
            self.consecutive += 1;
        } else {
            self.last_line = Some(line);
            self.consecutive = 1;
        }
        self.consecutive >= STREAM_REPEAT_THRESHOLD
    }
}

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
    /// `true` because most OpenAI-compatible providers (OpenAI, Anthropic
    /// adapters, GLM, Groq, Mistral, OpenHuman backend, …) implement the
    /// `tools` parameter correctly. The factory flips this to `false` for
    /// Ollama (sub-issue 3 of #3098), whose OpenAI-compat endpoint returns
    /// HTTP 400 on `tools` for many models — making prompt-guided text
    /// tool specs the only path that works across the Ollama model zoo.
    native_tool_calling: bool,
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

    fn enrich_404_message(&self, base: String, status: reqwest::StatusCode) -> String {
        if status == reqwest::StatusCode::NOT_FOUND && !self.supports_responses_fallback {
            format!(
                "{base}; check that your endpoint URL is correct \
                 and the model name exists on your provider"
            )
        } else {
            base
        }
    }

    /// Build an actionable error for a completion-only model that was routed
    /// to `/v1/chat/completions`. OpenHuman only speaks the chat-completions
    /// API (with an optional `/v1/responses` fallback) — a completion-only /
    /// base model 404s here and the responses fallback cannot rescue it, so we
    /// surface the model name and concrete remediation instead of an opaque
    /// "responses fallback failed" chain. See issue #3193.
    fn completion_only_model_message(&self, model: &str, sanitized: &str) -> String {
        format!(
            "{name} API error (404): model '{model}' does not support the \
             chat-completions API that OpenHuman uses — it appears to be a \
             completion-only / base model. Assign a chat-capable model to this \
             provider (e.g. in Settings → AI), or pick a different model. \
             Provider detail: {sanitized}",
            name = self.name,
        )
    }

    /// Guard shared by every chat-completions 404 handler: if the body shows a
    /// completion-only model, return the actionable error so the caller can
    /// fail fast instead of attempting the futile `/v1/responses` fallback.
    /// `None` means "not this case — proceed with normal fallback/enrich".
    /// See issue #3193.
    fn completion_only_404_guard(
        &self,
        status: reqwest::StatusCode,
        sanitized: &str,
        model: &str,
    ) -> Option<anyhow::Error> {
        if Self::is_completion_only_model_404(status, sanitized) {
            Some(anyhow::anyhow!(
                self.completion_only_model_message(model, sanitized)
            ))
        } else {
            None
        }
    }

    /// Build an actionable error for a model that lacks the chat capability —
    /// e.g. an *embedding* model (Ollama `bge-m3`) selected as the chat model.
    /// Ollama returns `400 "<model>" does not support chat`; we replace the
    /// opaque upstream JSON with concrete remediation. See Sentry
    /// TAURI-RUST-4P6.
    ///
    /// The phrase `does not support chat` is preserved verbatim so the
    /// re-reported error still matches
    /// [`super::config_rejection::is_provider_config_rejection_message`] and
    /// stays demoted from Sentry.
    fn not_chat_capable_model_message(&self, model: &str, sanitized: &str) -> String {
        format!(
            "{name} API error: model '{model}' does not support chat — it \
             appears to be an embedding or non-chat model. Assign a \
             chat-capable model to this provider (e.g. in Settings → AI), or \
             pick a different model. Provider detail: {sanitized}",
            name = self.name,
        )
    }

    /// Detect a model rejected because it has no chat capability. Unlike the
    /// completion-only base model (which 404s), an embedding model picked as
    /// the chat model is rejected by Ollama with a **400/422** carrying
    /// `"<model>" does not support chat`, so it bypasses
    /// [`is_completion_only_model_404`]. Match is tight (the exact phrase) so
    /// ordinary 400s keep their normal handling. See Sentry TAURI-RUST-4P6.
    fn is_not_chat_capable_model(status: reqwest::StatusCode, error: &str) -> bool {
        if !matches!(
            status,
            reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ) {
            return false;
        }
        error.to_lowercase().contains("does not support chat")
    }

    /// Guard shared by every chat-completions error handler: if the body shows
    /// a non-chat-capable model (embedding model picked as chat), return the
    /// actionable error so the caller fails fast with concrete remediation
    /// instead of surfacing the opaque upstream JSON. `None` means "not this
    /// case — proceed with normal fallback/enrich". See Sentry TAURI-RUST-4P6.
    fn not_chat_capable_guard(
        &self,
        status: reqwest::StatusCode,
        sanitized: &str,
        model: &str,
    ) -> Option<anyhow::Error> {
        if Self::is_not_chat_capable_model(status, sanitized) {
            Some(anyhow::anyhow!(
                self.not_chat_capable_model_message(model, sanitized)
            ))
        } else {
            None
        }
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
            ollama_num_ctx: None,
            local_provider_kind: None,
        }
    }

    /// Toggle whether this provider advertises native (OpenAI-style) tool
    /// calling to the agent harness. The default is `true`; set to `false`
    /// for providers whose `/v1/chat/completions` endpoint rejects the
    /// `tools` parameter — the harness will then embed tool specs in the
    /// system prompt and parse calls out of the response text instead.
    pub fn with_native_tool_calling(mut self, enabled: bool) -> Self {
        self.native_tool_calling = enabled;
        self
    }

    /// Set the list of model glob patterns for which temperature must be
    /// omitted from request bodies. Called by the provider factory to
    /// propagate `config.temperature_unsupported_models`.
    pub fn with_temperature_unsupported_models(mut self, patterns: Vec<String>) -> Self {
        self.temperature_unsupported_models = patterns;
        self
    }

    /// Pin a per-workload temperature, overriding whatever the caller passes.
    /// Set by the factory when the provider string carries an `@<temp>` suffix.
    pub fn with_temperature_override(mut self, temperature: Option<f64>) -> Self {
        self.temperature_override = temperature;
        self
    }

    /// Set the Ollama `options.num_ctx` override. When set, the provider
    /// includes `"options": {"num_ctx": <value>}` in every request body.
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

    async fn chat_via_responses(
        &self,
        credential: Option<&str>,
        messages: &[ChatMessage],
        model: &str,
    ) -> anyhow::Result<String> {
        let (instructions, input) = build_responses_prompt(messages);
        if input.is_empty() {
            anyhow::bail!(
                "{} Responses API fallback requires at least one non-system message",
                self.name
            );
        }

        // #3201: the Codex/ChatGPT OAuth Responses endpoint
        // (`https://chatgpt.com/backend-api/codex/responses`) rejects
        // `stream: false` outright with `{"detail":"Stream must be set to
        // true"}`. PR #3192 fixed the sibling `store: false` requirement;
        // this branch lifts the same constraint for the stream flag and
        // parses the resulting SSE body inline so the existing non-streaming
        // call signature is preserved. Other Responses-API providers (real
        // OpenAI, custom OpenAI-compatible) keep the single-envelope path —
        // they accept `stream: false` and the SSE branch would be wasted
        // work for them.
        //
        // Detection is keyed on the `/backend-api/codex` path segment, not
        // the `chatgpt.com` host: the same path segment is what
        // `OpenAiCodexRouting` substitutes when a user is signed in via
        // OAuth (see `OPENAI_CODEX_BACKEND_BASE_URL`), and it's specific
        // enough that no other OpenAI-compatible provider URL uses it.
        //
        // Parse the URL and inspect path segments rather than scanning the
        // whole `base_url` so a proxy URL whose query string or fragment
        // contains the literal `/backend-api/codex` (e.g.
        // `.../v1?upstream=/backend-api/codex`) doesn't get falsely
        // promoted into the SSE branch.
        let is_codex_oauth_responses = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| {
                let segments: Vec<&str> = url.path_segments()?.collect();
                Some(
                    segments
                        .windows(2)
                        .any(|window| window == ["backend-api", "codex"]),
                )
            })
            .unwrap_or(false);

        let request = ResponsesRequest {
            model: model.to_string(),
            input,
            instructions,
            stream: Some(is_codex_oauth_responses),
            store: Some(false),
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let status_str = status.as_u16().to_string();
            let error = response.text().await?;
            let sanitized = super::sanitize_api_error(&error);
            let message = format!("{} Responses API error: {sanitized}", self.name);
            if super::is_budget_exhausted_http_400(status, &error) {
                super::log_budget_exhausted_http_400(
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::log_custom_openai_upstream_bad_request_http_400(
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_access_policy_denied_http_403(status, &error) {
                super::log_provider_access_policy_denied_http_403(
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_config_rejection_http(status, self.name.as_str(), &error) {
                super::log_provider_config_rejection(
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "responses_api",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", model),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let body = response.text().await?;
        if is_codex_oauth_responses {
            // SSE branch — `stream: true` always produces a Server-Sent
            // Event body, even on the non-streaming wrapper. Aggregate it
            // back into the same `String` shape the caller expects.
            return aggregate_responses_sse_body(&self.name, &body);
        }
        let responses = parse_responses_response_body(&self.name, &body)?;

        extract_responses_text(responses)
            .ok_or_else(|| anyhow::anyhow!("No response from {} Responses API", self.name))
    }

    fn convert_tool_specs(
        tools: Option<&[crate::openhuman::tools::ToolSpec]>,
    ) -> Option<Vec<serde_json::Value>> {
        tools.map(|items| {
            let mut seen: std::collections::HashSet<&str> =
                std::collections::HashSet::with_capacity(items.len());
            let mut dropped: Vec<&str> = Vec::new();
            let mut out: Vec<serde_json::Value> = Vec::with_capacity(items.len());
            for tool in items {
                if !seen.insert(tool.name.as_str()) {
                    dropped.push(tool.name.as_str());
                    continue;
                }
                out.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                }));
            }
            if !dropped.is_empty() {
                log::warn!(
                    "[providers][compatible] dropped {} duplicate tool spec(s) at wire \
                     boundary (TAURI-RUST-2E): {:?}",
                    dropped.len(),
                    dropped
                );
            }
            out
        })
    }

    fn convert_messages_for_native(messages: &[ChatMessage]) -> Vec<NativeMessage> {
        let converted: Vec<NativeMessage> =
            messages
                .iter()
                .map(|message| {
                    // Extract reasoning_content stored in extra_metadata by the
                    // agent harness after each assistant turn. Thinking models
                    // (DeepSeek-R1, Qwen3, GLM-4) require this to be echoed back
                    // verbatim in subsequent requests, or the API returns HTTP 400.
                    let reasoning_content = if message.role == "assistant" {
                        message
                            .extra_metadata
                            .as_ref()
                            .and_then(|m| m.get("reasoning_content"))
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                    } else {
                        None
                    };

                    if message.role == "assistant" {
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&message.content)
                        {
                            if let Some(tool_calls_value) = value.get("tool_calls") {
                                if let Ok(parsed_calls) =
                                    serde_json::from_value::<Vec<ProviderToolCall>>(
                                        tool_calls_value.clone(),
                                    )
                                {
                                    let tool_calls = parsed_calls
                                        .into_iter()
                                        .map(|tc| ToolCall {
                                            id: Some(tc.id),
                                            kind: Some("function".to_string()),
                                            function: Some(Function {
                                                name: Some(tc.name),
                                                arguments: Some(serde_json::Value::String(
                                                    tc.arguments,
                                                )),
                                            }),
                                        })
                                        .collect::<Vec<_>>();

                                    // Default to empty string (not None) for
                                    // tool-call assistant messages so the wire
                                    // emits `"content":""` rather than omitting
                                    // the key — some providers reject a missing
                                    // content alongside reasoning_content.
                                    let content = Some(MessageContent::Text(
                                        value
                                            .get("content")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                    ));

                                    // Replay the assistant's reasoning so
                                    // DeepSeek thinking mode accepts the
                                    // tool-call turn on the follow-up request
                                    // (Sentry TAURI-RUST-4KB). Prefer the value
                                    // embedded in the JSON content (written by
                                    // `build_native_assistant_history` in the
                                    // tool-loop path); fall back to the value
                                    // stored in `extra_metadata` (written by the
                                    // main session-turn path).
                                    let reasoning_content = value
                                        .get("reasoning_content")
                                        .and_then(serde_json::Value::as_str)
                                        .filter(|s| !s.trim().is_empty())
                                        .map(ToString::to_string)
                                        .or_else(|| reasoning_content.clone());

                                    return NativeMessage {
                                        role: "assistant".to_string(),
                                        content,
                                        tool_call_id: None,
                                        tool_calls: Some(tool_calls),
                                        reasoning_content,
                                    };
                                }
                            }
                        }
                    }

                    if message.role == "tool" {
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&message.content)
                        {
                            let tool_call_id = value
                                .get("tool_call_id")
                                .and_then(serde_json::Value::as_str)
                                .map(ToString::to_string);
                            let content = value
                                .get("content")
                                .and_then(serde_json::Value::as_str)
                                .map(ToString::to_string)
                                .or_else(|| Some(message.content.clone()))
                                .map(MessageContent::Text);

                            return NativeMessage {
                                role: "tool".to_string(),
                                content,
                                tool_call_id,
                                tool_calls: None,
                                reasoning_content: None,
                            };
                        }
                    }

                    NativeMessage {
                        role: message.role.clone(),
                        // User-authored content may carry `[IMAGE:<data-uri>]`
                        // markers from chat attachments — promote them to
                        // structured `image_url` parts here. Markerless text
                        // (every system/assistant/tool turn) is returned as the
                        // plain-string arm, unchanged on the wire.
                        content: Some(MessageContent::from_chat_text(&message.content)),
                        tool_call_id: None,
                        tool_calls: None,
                        reasoning_content,
                    }
                })
                .collect();

        Self::enforce_tool_message_invariants(converted)
    }

    /// Enforce the OpenAI-compatible tool-message ordering invariants on the
    /// fully-serialized wire array, immediately before it goes on the wire.
    ///
    /// Several upstream defects can leave the array malformed and trip a 400
    /// (`messages with role 'tool' must be a response to a preceding message
    /// with 'tool_calls'`). That 400 streams back as an empty completion, which
    /// the agent loop collapses to "The model returned an empty response" and
    /// the chat surface shows as a generic "Something went wrong":
    ///
    /// * **(A)** History tail-trimming (`session::turn::trim_history` /
    ///   `bound_cached_transcript_messages`) cuts *between* an
    ///   `assistant(tool_calls)` and its `tool` result, dropping the assistant
    ///   and orphaning the result at the head of the window.
    /// * **(B)** A persisted assistant tool-call message whose `content` no
    ///   longer deserializes as `tool_calls` (format drift) falls through the
    ///   parser above and is emitted as plain text with its `tool_calls`
    ///   stripped — again orphaning the following `tool` result.
    /// * **(C)** An `assistant(tool_calls)` whose results never arrived (an
    ///   aborted / max-iteration turn, or a partially-answered multi-call
    ///   cycle) leaves dangling tool-call ids with no matching `tool` response.
    ///
    /// This pass makes the contract hold *by construction* regardless of which
    /// path produced the array. It is **position-aware**: each
    /// `assistant(tool_calls)` is paired with the *contiguous run of `tool`
    /// messages that immediately follows it* (the only place valid responses can
    /// live in the OpenAI wire format), then:
    ///
    /// * `tool_calls` entries with no matching response *in that run* are pruned
    ///   (C); if none survive, the field is dropped so the message serializes as
    ///   plain assistant text rather than an empty tool-call block.
    /// * `tool` messages that are **not** part of such a run — a leading orphan
    ///   from trimming (A), or one stranded after an assistant whose `tool_calls`
    ///   were stripped (B) — are dropped.
    ///
    /// Pairing by adjacency (rather than a global "is this id answered anywhere"
    /// set) is what keeps **sequential** cycles (`asst(A)→tool(A)`,
    /// `asst(B)→tool(B)`, …) and **parallel** calls (one `asst([X,Y,Z])` answered
    /// by `tool(X) tool(Y) tool(Z)`) correct, and makes the result well-formed
    /// even if responses are reordered or a cycle is bisected mid-sequence — no
    /// causal-ordering assumption required.
    fn enforce_tool_message_invariants(messages: Vec<NativeMessage>) -> Vec<NativeMessage> {
        use std::collections::HashSet;

        let mut out: Vec<NativeMessage> = Vec::with_capacity(messages.len());
        let mut dropped_orphans = 0usize;
        let mut pruned_calls = 0usize;

        let mut iter = messages.into_iter().peekable();
        while let Some(mut msg) = iter.next() {
            if msg.role == "assistant" && msg.tool_calls.is_some() {
                // Gather the contiguous run of `tool` messages that answer this
                // block (responses must immediately follow, in any order).
                let mut run: Vec<NativeMessage> = Vec::new();
                while iter.peek().is_some_and(|m| m.role == "tool") {
                    run.push(iter.next().expect("peeked tool message"));
                }
                let responded: HashSet<String> =
                    run.iter().filter_map(|t| t.tool_call_id.clone()).collect();

                // (C) keep only tool_calls answered within this run.
                let calls = msg.tool_calls.take().unwrap_or_default();
                let before = calls.len();
                let kept: Vec<ToolCall> = calls
                    .into_iter()
                    .filter(|c| c.id.as_deref().is_some_and(|id| responded.contains(id)))
                    .collect();
                pruned_calls += before - kept.len();
                let kept_ids: HashSet<String> = kept.iter().filter_map(|c| c.id.clone()).collect();
                msg.tool_calls = if kept.is_empty() { None } else { Some(kept) };
                // Strip reasoning_content when the message collapses to plain
                // text (no surviving tool_calls). Thinking-mode providers
                // (DeepSeek) require reasoning only on tool-call assistant
                // messages; a stale reasoning_content on a non-tool-call
                // message is at best ignored and at worst a malformed shape.
                if msg.tool_calls.is_none() {
                    msg.reasoning_content = None;
                }
                out.push(msg);

                // Emit the run's responses that map to a surviving call; drop the
                // rest (e.g. a stray tool whose id wasn't in this block).
                for tool_msg in run {
                    let kept = tool_msg
                        .tool_call_id
                        .as_deref()
                        .is_some_and(|id| kept_ids.contains(id));
                    if kept {
                        out.push(tool_msg);
                    } else {
                        dropped_orphans += 1;
                    }
                }
            } else if msg.role == "tool" {
                // (A, B) a `tool` not consumed by a preceding assistant block.
                dropped_orphans += 1;
            } else {
                out.push(msg);
            }
        }

        if dropped_orphans > 0 || pruned_calls > 0 {
            log::warn!(
                "[provider] sanitized malformed tool-message ordering before send: \
                 dropped {dropped_orphans} orphaned tool result(s), pruned {pruned_calls} \
                 unanswered tool_call(s)"
            );
        }

        out
    }

    fn with_prompt_guided_tool_instructions(
        messages: &[ChatMessage],
        tools: Option<&[crate::openhuman::tools::ToolSpec]>,
    ) -> Vec<ChatMessage> {
        let Some(tools) = tools else {
            return messages.to_vec();
        };

        if tools.is_empty() {
            return messages.to_vec();
        }

        let instructions =
            crate::openhuman::inference::provider::traits::build_tool_instructions_text(tools);
        let mut modified_messages = messages.to_vec();

        if let Some(system_message) = modified_messages.iter_mut().find(|m| m.role == "system") {
            if !system_message.content.is_empty() {
                system_message.content.push_str("\n\n");
            }
            system_message.content.push_str(&instructions);
        } else {
            modified_messages.insert(0, ChatMessage::system(instructions));
        }

        modified_messages
    }

    fn parse_native_response(
        api_response: ApiChatResponse,
        provider_name: &str,
    ) -> anyhow::Result<ProviderChatResponse> {
        let usage = Self::extract_usage(&api_response);

        let message = api_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow::anyhow!("No choices in response from {}", provider_name))?;

        let mut text = message.effective_content_optional();
        // Capture reasoning_content before the message fields are moved into
        // the tool-call extractors below. This must be passed back verbatim on
        // the next turn for thinking models (e.g. DeepSeek-R1, Qwen3) whose APIs
        // return HTTP 400 ("reasoning_content in thinking mode must be passed back")
        // when the field is omitted from subsequent assistant messages.
        let reasoning_content = message
            .reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let function = tc.function?;
                let name = function.name?;
                let arguments = normalize_function_arguments(function.arguments);
                Some(ProviderToolCall {
                    id: tc.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name,
                    arguments,
                })
            })
            .collect::<Vec<_>>();

        if tool_calls.is_empty() {
            if let Some(function) = message.function_call.as_ref() {
                if let Some(name) = function
                    .name
                    .as_ref()
                    .filter(|name| !name.trim().is_empty())
                {
                    tool_calls.push(ProviderToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: name.clone(),
                        arguments: normalize_function_arguments(function.arguments.clone()),
                    });
                }
            }
        }

        // Some providers return OpenAI-style tool_calls encoded as a JSON string
        // inside message.content. Recover those here so native tool-calling still works.
        if let Some(content) = message.content.as_deref() {
            if let Some((json_text, json_tool_calls)) = parse_tool_calls_from_content_json(content)
            {
                if !json_tool_calls.is_empty() {
                    tool_calls = json_tool_calls;
                    text = json_text.or(text);
                }
            }
        }

        tracing::debug!(
            has_reasoning_content = reasoning_content.is_some(),
            reasoning_content_chars = reasoning_content.as_ref().map_or(0, |r| r.chars().count()),
            "[provider:parse_native_response] reasoning_content capture"
        );

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
            reasoning_content,
        })
    }

    /// Extract usage info from API response, preferring the OpenHuman
    /// metadata block (which includes cache stats and billing) over the
    /// standard OpenAI usage block.
    fn extract_usage(resp: &ApiChatResponse) -> Option<ProviderUsageInfo> {
        let oh = resp.openhuman.as_ref();
        let std_usage = resp.usage.as_ref();

        // Need at least one source of token counts.
        if oh.is_none() && std_usage.is_none() {
            return None;
        }

        let oh_usage = oh.and_then(|o| o.usage.as_ref());
        let oh_billing = oh.and_then(|o| o.billing.as_ref());

        // Prefer OpenHuman metadata when the fields are actually present;
        // fall back to the standard OpenAI usage block when they are None.
        let input_tokens = oh_usage
            .and_then(|u| u.input_tokens)
            .or(std_usage.map(|u| u.prompt_tokens))
            .unwrap_or(0);
        let output_tokens = oh_usage
            .and_then(|u| u.output_tokens)
            .or(std_usage.map(|u| u.completion_tokens))
            .unwrap_or(0);
        let cached_input_tokens = oh_usage
            .and_then(|u| u.cached_input_tokens)
            .or(std_usage
                .and_then(|u| u.prompt_tokens_details.as_ref())
                .map(|d| d.cached_tokens))
            .unwrap_or(0);
        let charged_amount_usd = oh_billing.map(|b| b.charged_amount_usd).unwrap_or(0.0);

        let from_openhuman = oh_usage.is_some();
        let from_standard = std_usage.is_some() && !from_openhuman;
        let has_billing = oh_billing.is_some();
        tracing::debug!(
            from_openhuman,
            from_standard,
            has_billing,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
            "[provider:usage] extract_usage resolved token counts"
        );

        Some(ProviderUsageInfo {
            input_tokens,
            output_tokens,
            context_window: 0,
            cached_input_tokens,
            charged_amount_usd,
        })
    }

    fn is_native_tool_schema_unsupported(status: reqwest::StatusCode, error: &str) -> bool {
        if !matches!(
            status,
            reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ) {
            return false;
        }

        let lower = error.to_lowercase();
        [
            "unknown parameter: tools",
            "unsupported parameter: tools",
            "unrecognized field `tools`",
            "does not support tools",
            "function calling is not supported",
            "tool_choice",
        ]
        .iter()
        .any(|hint| lower.contains(hint))
    }

    fn err_supports_no_tools_retry(error: &str) -> bool {
        Self::is_native_tool_schema_unsupported(reqwest::StatusCode::BAD_REQUEST, error)
    }

    /// Detect a provider rejecting the `frequency_penalty` sampling field. Some
    /// strict OpenAI-compatible backends 400 on unknown params; when this fires
    /// the caller retries once with the field omitted (mirrors the no-tools
    /// retry). String-based because the streamed transport error surfaces the
    /// API error body.
    fn err_indicates_frequency_penalty_unsupported(error: &str) -> bool {
        let lower = error.to_lowercase();
        lower.contains("frequency_penalty")
            && (lower.contains("unsupported")
                || lower.contains("unknown")
                || lower.contains("unrecognized")
                || lower.contains("not supported")
                || lower.contains("does not support")
                || lower.contains("invalid")
                || lower.contains("unexpected"))
    }

    /// Detect a 404 whose body says the model is completion-only and cannot be
    /// served from `/v1/chat/completions` (OpenAI: "This is not a chat model
    /// and thus not supported in the v1/chat/completions endpoint. Did you
    /// mean to use v1/completions?"). When this fires, attempting the
    /// `/v1/responses` fallback is futile, so callers should fail fast with an
    /// actionable message via [`completion_only_model_message`]. The match is
    /// deliberately tight so ordinary "model does not exist" 404s are NOT
    /// caught (those should keep their existing fallback / enrich behaviour).
    /// See issue #3193.
    fn is_completion_only_model_404(status: reqwest::StatusCode, error: &str) -> bool {
        if status != reqwest::StatusCode::NOT_FOUND {
            return false;
        }
        let lower = error.to_lowercase();
        lower.contains("not a chat model")
            || (lower.contains("v1/chat/completions") && lower.contains("v1/completions"))
    }

    /// Streaming variant of the native-tools chat path.
    ///
    /// Sends the request with `stream: true`, consumes the upstream SSE
    /// stream chunk by chunk, forwards fine-grained `ProviderDelta`
    /// events to the caller-supplied sender, and returns the aggregated
    /// [`ProviderChatResponse`] once the stream ends. Per-chunk parsing
    /// uses [`StreamChunkResponse`] — a permissive subset of the
    /// OpenAI/Fireworks streaming schema that tolerates unknown fields.
    async fn stream_native_chat(
        &self,
        credential: Option<&str>,
        native_request: &NativeChatRequest,
        delta_tx: &tokio::sync::mpsc::Sender<crate::openhuman::inference::provider::ProviderDelta>,
        dump_seq: u64,
    ) -> anyhow::Result<ProviderChatResponse> {
        use futures_util::StreamExt;

        let url = self.chat_completions_url();
        log::info!(
            "[stream] {} POST {} (stream=true, tools={})",
            self.name,
            url,
            native_request.tools.as_ref().map_or(0, |t| t.len()),
        );

        let response = self
            .apply_auth_header(
                self.http_client()
                    .post(&url)
                    .header("Accept", "text/event-stream")
                    .json(native_request),
                credential,
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let status_str = status.as_u16().to_string();
            let body = response.text().await.unwrap_or_default();
            // Sanitize the upstream error body so we don't leak user
            // prompts, tool arguments, or credentials the backend
            // echoed back into the anyhow chain / logs.
            let sanitized = super::sanitize_api_error(&body);
            let message = format!(
                "{} streaming API error ({}): {}",
                self.name, status, sanitized
            );
            if super::is_budget_exhausted_http_400(status, &body) {
                super::log_budget_exhausted_http_400(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &body,
            ) {
                super::log_custom_openai_upstream_bad_request_http_400(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::is_provider_access_policy_denied_http_403(status, &body) {
                super::log_provider_access_policy_denied_http_403(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::is_provider_config_rejection_http(status, self.name.as_str(), &body) {
                super::log_provider_config_rejection(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if Self::is_native_tool_schema_unsupported(status, &body) {
                // Model rejects tool definitions (e.g. Ollama "does not support tools").
                // The caller's retry loop already handles this by re-issuing without
                // tools — suppress the Sentry event so noise doesn't accumulate for
                // every model that lacks tool-calling support (TAURI-RUST-4K7).
                log::info!(
                    "[stream] {} model rejected tool schema (status={}) — caller will retry without tools",
                    self.name,
                    status,
                );
            } else if super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "streaming_chat",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", native_request.model.as_str()),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        // Some OpenAI-compatible backends (and our e2e mock) accept
        // `stream: true` in the request but reply with a regular
        // `application/json` body rather than SSE. Detect this and
        // fall back to the non-streaming parse path so the caller
        // still gets an aggregated response. No deltas are emitted in
        // this case (there's nothing to stream).
        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
            .unwrap_or(false);
        if !is_sse {
            log::warn!(
                "[stream] {} upstream replied with non-SSE content-type; falling back to JSON parse \
                 (no token deltas reach the UI)",
                self.name,
            );
            let response_bytes = response.bytes().await?;
            dump_response_if_enabled(&self.name, &native_request.model, dump_seq, &response_bytes);
            let api_resp: ApiChatResponse = serde_json::from_slice(&response_bytes)
                .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;
            return Self::parse_native_response(api_resp, &self.name);
        }

        // Accumulators for the final aggregated response. Tool-call
        // state is keyed by the upstream `index` so interleaved chunks
        // for multiple tool calls in the same turn don't clobber each
        // other.
        let mut text_accum = String::new();
        let mut thinking_accum = String::new();
        let mut tool_accum: std::collections::BTreeMap<u32, StreamingToolCall> =
            std::collections::BTreeMap::new();
        let mut last_usage: Option<ApiUsage> = None;
        let mut last_openhuman: Option<OpenHumanMeta> = None;

        let mut bytes_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut repeat_detector = StreamRepeatDetector::new();
        let mut degenerate_repeat = false;

        'stream: while let Some(item) = bytes_stream.next().await {
            let bytes = item?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // SSE events are separated by "\n\n"; lines within an event
            // are "\n"-terminated. We accumulate partial events across
            // socket reads and only pop complete ones.
            while let Some(sep_idx) = buffer.find("\n\n") {
                let event = buffer[..sep_idx].to_string();
                buffer.drain(..sep_idx + 2);
                for line in event.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        continue;
                    }

                    let chunk: StreamChunkResponse = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            log::debug!(
                                "[stream] {} skipping unparseable chunk: {} — data={}",
                                self.name,
                                e,
                                data,
                            );
                            continue;
                        }
                    };

                    if let Some(usage) = chunk.usage {
                        last_usage = Some(usage);
                    }
                    if let Some(meta) = chunk.openhuman {
                        last_openhuman = Some(meta);
                    }

                    for choice in chunk.choices {
                        // Visible text delta.
                        if let Some(content) = choice.delta.content.as_ref() {
                            if !content.is_empty() {
                                text_accum.push_str(content);
                                let _ = delta_tx
                                    .send(crate::openhuman::inference::provider::ProviderDelta::TextDelta {
                                        delta: content.clone(),
                                    })
                                    .await;
                                // Deterministic in-generation repeat cutoff: a
                                // model spiraling on one line can't be stopped by
                                // prompt or penalty, so abort the stream once the
                                // same substantial line repeats too many times.
                                if repeat_detector.observe(content) {
                                    log::warn!(
                                        "[stream] {} degenerate repetition detected (≥{} identical lines) — aborting generation, truncating (text_chars={})",
                                        self.name,
                                        STREAM_REPEAT_THRESHOLD,
                                        text_accum.chars().count(),
                                    );
                                    degenerate_repeat = true;
                                    break 'stream;
                                }
                            }
                        }
                        // Reasoning / thinking delta.
                        if let Some(reasoning) = choice.delta.reasoning_content.as_ref() {
                            if !reasoning.is_empty() {
                                thinking_accum.push_str(reasoning);
                                let _ = delta_tx
                                    .send(
                                        crate::openhuman::inference::provider::ProviderDelta::ThinkingDelta {
                                            delta: reasoning.clone(),
                                        },
                                    )
                                    .await;
                            }
                        }
                        // Tool-call fragments.
                        //
                        // Ordering invariant emitted downstream:
                        //   ToolCallStart (once, when id+name both known)
                        //     → ToolCallArgsDelta* (buffered then streamed)
                        //
                        // Args fragments that arrive *before* we know the
                        // canonical id are buffered into `entry.arguments`
                        // but NOT emitted — emitting them with a synthetic
                        // id would break client-side reconciliation against
                        // the eventual tool_call / tool_result events that
                        // carry the real id. Once start fires we flush the
                        // buffered prefix in a single delta, then stream
                        // subsequent fragments as they arrive.
                        if let Some(tc_list) = choice.delta.tool_calls.as_ref() {
                            for tc in tc_list {
                                let idx = tc.index.unwrap_or(0);
                                let entry = tool_accum.entry(idx).or_default();

                                if let Some(id) = tc.id.as_ref() {
                                    if entry.id.is_none() {
                                        log::debug!(
                                            "[stream] {} tool_call[{}] id resolved: {}",
                                            self.name,
                                            idx,
                                            id,
                                        );
                                    }
                                    entry.id = Some(id.clone());
                                }
                                if let Some(func) = tc.function.as_ref() {
                                    if let Some(name) = func.name.as_ref() {
                                        if !name.is_empty() && entry.name.is_none() {
                                            log::debug!(
                                                "[stream] {} tool_call[{}] name resolved: {}",
                                                self.name,
                                                idx,
                                                name,
                                            );
                                        }
                                        if !name.is_empty() {
                                            entry.name = Some(name.clone());
                                        }
                                    }
                                    if let Some(args) = func.arguments.as_ref() {
                                        if !args.is_empty() {
                                            entry.arguments.push_str(args);
                                            if !entry.emitted_start {
                                                log::debug!(
                                                    "[stream] {} tool_call[{}] buffering args ({} chars total) — waiting for id/name",
                                                    self.name,
                                                    idx,
                                                    entry.arguments.len(),
                                                );
                                            }
                                        }
                                    }
                                }

                                // Fire start + flush buffered args once
                                // both id and name have been observed.
                                if !entry.emitted_start {
                                    if let (Some(id), Some(name)) =
                                        (entry.id.as_ref(), entry.name.as_ref())
                                    {
                                        log::debug!(
                                            "[stream] {} tool_call[{}] emitting ToolCallStart id={} name={}",
                                            self.name,
                                            idx,
                                            id,
                                            name,
                                        );
                                        let _ = delta_tx
                                            .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallStart {
                                                call_id: id.clone(),
                                                tool_name: name.clone(),
                                            })
                                            .await;
                                        entry.emitted_start = true;
                                        // Flush any args that were
                                        // buffered before the start id
                                        // was known.
                                        if !entry.arguments.is_empty() {
                                            log::debug!(
                                                "[stream] {} tool_call[{}] flushing buffered args ({} chars)",
                                                self.name,
                                                idx,
                                                entry.arguments.len(),
                                            );
                                            let buffered = entry.arguments.clone();
                                            let _ = delta_tx
                                                .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallArgsDelta {
                                                    call_id: id.clone(),
                                                    delta: buffered,
                                                })
                                                .await;
                                            entry.emitted_chars = entry.arguments.len();
                                        }
                                    }
                                } else if entry.arguments.len() > entry.emitted_chars {
                                    // Start already fired — stream the
                                    // newly appended fragment with the
                                    // canonical id.
                                    if let Some(ref id) = entry.id {
                                        let fresh =
                                            entry.arguments[entry.emitted_chars..].to_string();
                                        let _ = delta_tx
                                            .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallArgsDelta {
                                                call_id: id.clone(),
                                                delta: fresh,
                                            })
                                            .await;
                                        entry.emitted_chars = entry.arguments.len();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if degenerate_repeat {
            // Mark the truncated output so downstream (and the user) see why it
            // was cut off rather than a silently shortened response.
            text_accum.push_str(
                "\n\n[Output stopped: detected repeated/looping generation (model degeneration).]",
            );
        }

        let tool_call_count = tool_accum.len();
        log::info!(
            "[stream] {} aggregated text_chars={} thinking_chars={} tool_calls={}",
            self.name,
            text_accum.chars().count(),
            thinking_accum.chars().count(),
            tool_call_count,
        );

        // Aggregate the collected tool calls into the unified response
        // shape. We reuse `parse_native_response` by building an
        // `ApiChatResponse` from the accumulators so downstream code
        // sees the same shape as the non-streaming path.
        let tool_calls_for_api: Vec<ToolCall> = tool_accum
            .into_values()
            .map(|c| ToolCall {
                id: c.id,
                kind: Some("function".to_string()),
                function: Some(Function {
                    name: c.name,
                    arguments: if c.arguments.is_empty() {
                        None
                    } else {
                        // Try to parse as JSON first so downstream
                        // `normalize_function_arguments` can take the
                        // usual Value (object) path; fall back to a
                        // JSON-string value for partially-assembled or
                        // permanently malformed fragments.
                        // `normalize_function_arguments` validates and
                        // discards malformed strings (OPENHUMAN-TAURI-6F).
                        Some(
                            serde_json::from_str(&c.arguments)
                                .unwrap_or(serde_json::Value::String(c.arguments)),
                        )
                    },
                }),
            })
            .collect();

        let api_resp = ApiChatResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: if text_accum.is_empty() {
                        None
                    } else {
                        Some(text_accum)
                    },
                    reasoning_content: if thinking_accum.is_empty() {
                        None
                    } else {
                        Some(thinking_accum)
                    },
                    tool_calls: if tool_calls_for_api.is_empty() {
                        None
                    } else {
                        Some(tool_calls_for_api)
                    },
                    function_call: None,
                },
            }],
            usage: last_usage,
            openhuman: last_openhuman,
        };

        // Dump the aggregated final response (structured, diff-friendly,
        // carries usage + openhuman cache meta from the last chunks).
        // Hand-build a Value here because `ApiChatResponse` is
        // Deserialize-only.
        if std::env::var("OPENHUMAN_PROMPT_DUMP_DIR").is_ok() {
            let msg = &api_resp.choices[0].message;
            let aggregated = serde_json::json!({
                "content": msg.content,
                "reasoning_content": msg.reasoning_content,
                "tool_calls": msg.tool_calls.as_ref().map(|calls| {
                    calls.iter().map(|c| serde_json::json!({
                        "id": c.id,
                        "type": c.kind,
                        "function": c.function.as_ref().map(|f| serde_json::json!({
                            "name": f.name,
                            "arguments": f.arguments,
                        })),
                    })).collect::<Vec<_>>()
                }),
                "usage": api_resp.usage.as_ref().map(|u| serde_json::json!({
                    "prompt_tokens": u.prompt_tokens,
                    "completion_tokens": u.completion_tokens,
                    "total_tokens": u.total_tokens,
                    "prompt_cached_tokens": u.prompt_tokens_details
                        .as_ref().map(|d| d.cached_tokens),
                })),
                "openhuman": api_resp.openhuman.as_ref().map(|m| serde_json::json!({
                    "usage": m.usage.as_ref().map(|u| serde_json::json!({
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "cached_input_tokens": u.cached_input_tokens,
                    })),
                    "billing": m.billing.as_ref().map(|b| serde_json::json!({
                        "charged_amount_usd": b.charged_amount_usd,
                    })),
                })),
            });
            if let Ok(bytes) = serde_json::to_vec(&aggregated) {
                dump_response_if_enabled(&self.name, &native_request.model, dump_seq, &bytes);
            }
        }

        Self::parse_native_response(api_resp, &self.name)
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn capabilities(&self) -> crate::openhuman::inference::provider::traits::ProviderCapabilities {
        crate::openhuman::inference::provider::traits::ProviderCapabilities {
            native_tool_calling: self.native_tool_calling,
            // Kept `false` for now. The provider already serializes images as
            // `image_url` content parts on the chat-completions path (#3205), but
            // vision is a per-*model* property the provider can't know here — and
            // the Responses-API path (`chat_via_responses`) is still text-only.
            // Claiming vision provider-wide would let image turns through the
            // gate to a possibly-non-vision model. The capability stays off until
            // it can be driven per-model (e.g. from `model_registry.vision`).
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential_for_request()?;

        let mut messages = Vec::new();

        if self.merge_system_into_user {
            let content = match system_prompt {
                Some(sys) => format!("{sys}\n\n{message}"),
                None => message.to_string(),
            };
            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::from_chat_text(&content),
            });
        } else {
            if let Some(sys) = system_prompt {
                messages.push(Message {
                    role: "system".to_string(),
                    content: sys.into(),
                });
            }
            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::from_chat_text(message),
            });
        }

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();

        let mut fallback_messages = Vec::new();
        if let Some(system_prompt) = system_prompt {
            fallback_messages.push(ChatMessage::system(system_prompt));
        }
        fallback_messages.push(ChatMessage::user(message));
        let fallback_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(&fallback_messages)
        } else {
            fallback_messages
        };

        if self.responses_api_primary {
            return self
                .chat_via_responses(credential, &fallback_messages, model)
                .await;
        }

        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(credential, &fallback_messages, model)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} chat completions transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            let sanitized = super::sanitize_api_error(&error);

            // A completion-only model 404s here and the /v1/responses fallback
            // cannot rescue it — fail fast with actionable guidance (#3193).
            if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                return Err(err);
            }

            // An embedding / non-chat model rejected with 400 "does not
            // support chat" (e.g. Ollama bge-m3 picked as the chat model) —
            // fail fast with actionable guidance. See Sentry TAURI-RUST-4P6.
            if let Some(err) = self.not_chat_capable_guard(status, &sanitized, model) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &fallback_messages, model)
                    .await
                    .map_err(|responses_err| {
                        let fb = super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            let status_str = status.as_u16().to_string();
            let message = self.enrich_404_message(
                format!("{} API error ({status}): {sanitized}", self.name),
                status,
            );
            if super::is_backend_auth_failure(self.name.as_str(), status) {
                // Backend rejected the app session JWT (401/403): expected
                // session-expiry (token expired/revoked/rotated), not a code
                // bug. Publish SessionExpired so the credentials subscriber
                // drives reauth and the scheduler-gate halts downstream LLM
                // work, and skip the Sentry report (TAURI-RUST-N). Mirrors the
                // `is_backend_auth_failure` arm in `super::api_error`.
                super::publish_backend_session_expired(
                    "chat_completions",
                    self.name.as_str(),
                    status,
                    &message,
                );
            } else if super::is_budget_exhausted_http_400(status, &error) {
                super::log_budget_exhausted_http_400(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::log_custom_openai_upstream_bad_request_http_400(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_access_policy_denied_http_403(status, &error) {
                super::log_provider_access_policy_denied_http_403(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_config_rejection_http(status, self.name.as_str(), &error) {
                super::log_provider_config_rejection(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "chat_completions",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", model),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let body = response.text().await?;
        let chat_response = parse_chat_response_body(&self.name, &body)?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| {
                // If tool_calls are present, serialize the full message as JSON
                // so parse_tool_calls can handle the OpenAI-style format
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    // No tool calls, return content (with reasoning_content fallback)
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential_for_request()?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: MessageContent::from_chat_text(&m.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        if self.responses_api_primary {
            return self
                .chat_via_responses(credential, &effective_messages, model)
                .await;
        }

        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(credential, &effective_messages, model)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} chat completions transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();

            // A 404 may mean this provider uses the Responses API, OR that the
            // model is completion-only. Read the body once so we can tell the
            // two apart (#3193) — only the 404 branch needs it; the response is
            // not used again here, so `api_error` below still owns the rest.
            if status == reqwest::StatusCode::NOT_FOUND {
                let error = response.text().await?;
                let sanitized = super::sanitize_api_error(&error);

                // Completion-only model: the responses fallback can't help —
                // fail fast with actionable guidance.
                if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                    return Err(err);
                }

                if self.supports_responses_fallback {
                    return self
                        .chat_via_responses(credential, &effective_messages, model)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                let enriched = self.enrich_404_message(
                    format!("{} API error ({status}): {sanitized}", self.name),
                    status,
                );
                return Err(anyhow::anyhow!("{enriched}"));
            }

            // `api_error` reads the body and runs the shared classification
            // (SessionExpired publish, config-rejection demotion, Sentry-report
            // decision). For a non-chat-capable model (embedding model picked
            // as chat → 400 "does not support chat") it already demotes the
            // event, but its message is the opaque upstream JSON. Upgrade that
            // to the actionable "assign a chat-capable model" copy — which
            // still carries the phrase, so it stays demoted on any re-report.
            // See Sentry TAURI-RUST-4P6.
            let err = super::api_error(&self.name, response).await;
            let err_str = err.to_string();
            if Self::is_not_chat_capable_model(status, &err_str) {
                return Err(anyhow::anyhow!(
                    self.not_chat_capable_model_message(model, &err_str)
                ));
            }
            let enriched = self.enrich_404_message(format!("{err:#}"), status);
            return Err(anyhow::anyhow!("{enriched}"));
        }

        let body = response.text().await?;
        let chat_response = parse_chat_response_body(&self.name, &body)?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| {
                // If tool_calls are present, serialize the full message as JSON
                // so parse_tool_calls can handle the OpenAI-style format
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    // No tool calls, return content (with reasoning_content fallback)
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential_for_request()?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: MessageContent::from_chat_text(&m.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.to_vec())
            },
            tool_choice: if tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "{} native tool call transport failed: {error}; falling back to history path",
                    self.name
                );
                let text = self.chat_with_history(messages, model, temperature).await?;
                return Ok(ProviderChatResponse {
                    text: Some(text),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }
        };

        if !response.status().is_success() {
            return Err(super::api_error(&self.name, response).await);
        }

        let body = response.text().await?;
        let chat_response = parse_chat_response_body(&self.name, &body)?;
        let usage = Self::extract_usage(&chat_response);
        let choice = chat_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))?;

        let text = choice.message.effective_content_optional();
        // See `parse_native_response`: replay reasoning on the follow-up
        // request so DeepSeek thinking mode accepts the tool-call turn.
        let reasoning_content = choice
            .message
            .reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let function = tc.function?;
                let name = function.name?;
                let arguments = normalize_function_arguments(function.arguments);
                Some(ProviderToolCall {
                    id: tc.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name,
                    arguments,
                })
            })
            .collect::<Vec<_>>();

        tracing::debug!(
            has_reasoning_content = reasoning_content.is_some(),
            reasoning_content_chars = reasoning_content.as_ref().map_or(0, |r| r.chars().count()),
            tool_calls = tool_calls.len(),
            "[provider:chat] reasoning_content capture (non-streaming)"
        );

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
            reasoning_content,
        })
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential_for_request()?;

        let tools = Self::convert_tool_specs(request.tools);
        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(request.messages)
        } else {
            request.messages.to_vec()
        };

        if self.responses_api_primary {
            let response_messages = if request.tools.is_some() {
                Self::with_prompt_guided_tool_instructions(request.messages, request.tools)
            } else {
                effective_messages.clone()
            };
            let text = self
                .chat_via_responses(credential, &response_messages, model)
                .await?;
            if let Some(tx) = request.stream {
                let _ = tx
                    .send(
                        crate::openhuman::inference::provider::ProviderDelta::TextDelta {
                            delta: text.clone(),
                        },
                    )
                    .await;
            }
            return Ok(ProviderChatResponse {
                text: Some(text),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            });
        }

        // ── Streaming branch ─────────────────────────────────────────
        // When the caller supplied a `ProviderDelta` sender, request
        // SSE and forward fine-grained deltas while accumulating the
        // final response. Fall back to non-streaming on non-200 errors
        // so tool-schema rejections etc. still work.
        if let Some(tx) = request.stream {
            let native_request = NativeChatRequest {
                model: model.to_string(),
                messages: Self::convert_messages_for_native(&effective_messages),
                temperature: self.effective_temperature(model, temperature),
                stream: Some(true),
                tool_choice: tools.as_ref().map(|_| "auto".to_string()),
                tools: tools.clone(),
                thread_id: self.outbound_thread_id(),
                // Ask the server for a final usage chunk so token
                // accounting (and `openhuman.billing.charged_amount_usd`
                // for the OpenHuman backend) makes it back from
                // streaming responses — orchestrator sessions otherwise
                // lose the `- Charged: $…` line in their transcripts.
                stream_options: Some(OpenAiStreamOptions {
                    include_usage: true,
                }),
                options: self.build_ollama_options(),
                frequency_penalty: Some(CHAT_FREQUENCY_PENALTY),
            };
            let stream_dump_seq = reserve_dump_seq();
            dump_prompt_if_enabled(&self.name, model, stream_dump_seq, &native_request);
            match self
                .stream_native_chat(credential, &native_request, tx, stream_dump_seq)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let err_str = err.to_string();
                    // Some local-runtime models (e.g. Ollama serving
                    // gemma3, llama3.2:1b, …) reject the request with
                    // "<model> does not support tools" when the
                    // ChatRequest carries a `tools` array. Retry the
                    // streaming call once with tools stripped so the
                    // user still gets a live token stream — without
                    // this we'd silently fall through to the buffered
                    // non-streaming path and the UI would render the
                    // reply all at once.
                    if tools.is_some() && Self::err_supports_no_tools_retry(&err_str) {
                        log::info!(
                            "[stream] {} model does not support tools — retrying streaming without tools",
                            self.name,
                        );
                        let retry_request = NativeChatRequest {
                            tools: None,
                            tool_choice: None,
                            ..native_request.clone()
                        };
                        match self
                            .stream_native_chat(credential, &retry_request, tx, stream_dump_seq)
                            .await
                        {
                            Ok(resp) => return Ok(resp),
                            Err(retry_err) => {
                                log::warn!(
                                    "[stream] {} retry without tools also failed, falling back to non-streaming: {}",
                                    self.name,
                                    retry_err
                                );
                            }
                        }
                    } else if Self::err_indicates_frequency_penalty_unsupported(&err_str) {
                        // Symmetric to the no-tools retry: a strict provider that
                        // 400s on `frequency_penalty` should degrade gracefully
                        // rather than fail the whole chat path.
                        log::info!(
                            "[stream] {} rejected frequency_penalty — retrying streaming without it",
                            self.name,
                        );
                        let retry_request = NativeChatRequest {
                            frequency_penalty: None,
                            ..native_request.clone()
                        };
                        match self
                            .stream_native_chat(credential, &retry_request, tx, stream_dump_seq)
                            .await
                        {
                            Ok(resp) => return Ok(resp),
                            Err(retry_err) => {
                                log::warn!(
                                    "[stream] {} retry without frequency_penalty also failed, falling back to non-streaming: {}",
                                    self.name,
                                    retry_err
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "[stream] {} streaming chat failed, falling back to non-streaming: {}",
                            self.name,
                            err
                        );
                    }
                    // Fall through to the non-streaming path below. The
                    // non-streaming request below omits `frequency_penalty` so a
                    // provider that rejected it (streaming or not) still succeeds.
                }
            }
        }

        let thread_id = self.outbound_thread_id();
        log::debug!(
            "[provider:{}] chat() outbound thread_id={} model={}",
            self.name,
            thread_id.as_deref().unwrap_or("<none>"),
            model
        );
        let native_request = NativeChatRequest {
            model: model.to_string(),
            messages: Self::convert_messages_for_native(&effective_messages),
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
            thread_id,
            stream_options: None,
            options: self.build_ollama_options(),
            // The buffered (non-streaming) path is the fallback / non-streaming
            // provider path — omit `frequency_penalty` here for maximum
            // compatibility (a provider that rejects it still succeeds). The
            // streaming path above carries it (where degenerate repetition loops
            // actually occur) and retries without it on rejection.
            frequency_penalty: None,
        };
        let dump_seq = reserve_dump_seq();
        dump_prompt_if_enabled(&self.name, model, dump_seq, &native_request);

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(
                self.http_client().post(&url).json(&native_request),
                credential,
            )
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(credential, &effective_messages, model)
                        .await
                        .map(|text| ProviderChatResponse {
                            text: Some(text),
                            tool_calls: vec![],
                            usage: None,
                            reasoning_content: None,
                        })
                        .map_err(|responses_err| {
                            let fb = super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} native chat transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            let sanitized = super::sanitize_api_error(&error);

            if Self::is_native_tool_schema_unsupported(status, &sanitized) {
                let fallback_messages =
                    Self::with_prompt_guided_tool_instructions(request.messages, request.tools);
                let text = self
                    .chat_with_history(&fallback_messages, model, temperature)
                    .await?;
                return Ok(ProviderChatResponse {
                    text: Some(text),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }

            // A completion-only model 404s here and the /v1/responses fallback
            // cannot rescue it — fail fast with actionable guidance (#3193).
            if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                return Err(err);
            }

            // An embedding / non-chat model rejected with 400 "does not
            // support chat" (e.g. Ollama bge-m3 picked as the chat model) —
            // fail fast with actionable guidance. See Sentry TAURI-RUST-4P6.
            if let Some(err) = self.not_chat_capable_guard(status, &sanitized, model) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &effective_messages, model)
                    .await
                    .map(|text| ProviderChatResponse {
                        text: Some(text),
                        tool_calls: vec![],
                        usage: None,
                        reasoning_content: None,
                    })
                    .map_err(|responses_err| {
                        let fb = super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            let status_str = status.as_u16().to_string();
            let message = self.enrich_404_message(
                format!("{} API error ({status}): {sanitized}", self.name),
                status,
            );
            if super::is_budget_exhausted_http_400(status, &error) {
                super::log_budget_exhausted_http_400(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::log_custom_openai_upstream_bad_request_http_400(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_access_policy_denied_http_403(status, &error) {
                super::log_provider_access_policy_denied_http_403(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::is_provider_config_rejection_http(status, self.name.as_str(), &error) {
                super::log_provider_config_rejection(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "native_chat",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", model),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let response_bytes = response.bytes().await?;
        dump_response_if_enabled(&self.name, model, dump_seq, &response_bytes);
        let native_response: ApiChatResponse = serde_json::from_slice(&response_bytes)
            .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;
        Self::parse_native_response(native_response, &self.name)
    }

    fn supports_native_tools(&self) -> bool {
        // Must mirror `capabilities().native_tool_calling`. Both signals are
        // read by the agent harness (`traits.rs:415`) to decide between an
        // OpenAI-style `tools` array and the prompt-guided text fallback;
        // letting them disagree would defeat `with_native_tool_calling(false)`
        // for the Ollama branch of sub-issue 3 of #3098.
        self.native_tool_calling
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let credential = match self.credential_for_request() {
            Ok(value) => value.map(str::to_string),
            Err(err) => {
                return stream::once(async move { Err(StreamError::Provider(err.to_string())) })
                    .boxed();
            }
        };

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system".to_string(),
                content: sys.into(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: MessageContent::from_chat_text(message),
        });

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(options.enabled),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        let client = self.http_client();
        let auth_header = self.auth_header.clone();
        let extra_headers = self.extra_headers.clone();
        let openrouter_attribution_headers = self.openrouter_attribution_headers();
        let provider_name = self.name.clone();
        let model_owned = model.to_string();

        // Use a channel to bridge the async HTTP response to the stream
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

        tokio::spawn(async move {
            // Build request with auth
            let mut req_builder = client.post(&url).json(&request);

            // Apply auth header
            req_builder = match (&auth_header, credential.as_deref()) {
                (AuthStyle::None, _) | (_, None) => req_builder,
                (AuthStyle::Bearer, Some(credential)) => {
                    req_builder.header("Authorization", format!("Bearer {credential}"))
                }
                (AuthStyle::XApiKey, Some(credential)) => {
                    req_builder.header("x-api-key", credential)
                }
                (AuthStyle::Anthropic, Some(credential)) => req_builder
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01"),
                (AuthStyle::Custom(header), Some(credential)) => {
                    req_builder.header(header, credential)
                }
            };

            for (name, value) in &extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
            if let Some((referer, title)) = openrouter_attribution_headers {
                req_builder = req_builder
                    .header("HTTP-Referer", referer)
                    .header("X-OpenRouter-Title", title);
            }

            // Set accept header for streaming
            req_builder = req_builder.header("Accept", "text/event-stream");

            // Send request
            let response = match req_builder.send().await {
                Ok(r) => r,
                Err(e) => {
                    crate::core::observability::report_error(
                        e.to_string().as_str(),
                        "llm_provider",
                        "stream_chat",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("failure", "transport"),
                        ],
                    );
                    let _ = tx.send(Err(StreamError::Http(e))).await;
                    return;
                }
            };

            // Check status
            if !response.status().is_success() {
                let status = response.status();
                let status_str = status.as_u16().to_string();
                let raw_error = match response.text().await {
                    Ok(e) => e,
                    Err(_) => format!("HTTP error: {}", status),
                };
                let sanitized_error = super::sanitize_api_error(&raw_error);
                let message = format!("{}: {}", status, sanitized_error);
                if super::is_budget_exhausted_http_400(status, &raw_error) {
                    super::log_budget_exhausted_http_400(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_custom_openai_upstream_bad_request_http_400(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    super::log_custom_openai_upstream_bad_request_http_400(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_provider_access_policy_denied_http_403(status, &raw_error) {
                    super::log_provider_access_policy_denied_http_403(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_provider_config_rejection_http(
                    status,
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    super::log_provider_config_rejection(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::should_report_provider_http_failure(status) {
                    crate::core::observability::report_error(
                        message.as_str(),
                        "llm_provider",
                        "stream_chat",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("status", status_str.as_str()),
                            ("failure", "non_2xx"),
                        ],
                    );
                }
                let _ = tx.send(Err(StreamError::Provider(message))).await;
                return;
            }

            // Convert to chunk stream and forward to channel
            let mut chunk_stream = sse_bytes_to_chunks(response, options.count_tokens);
            while let Some(chunk) = chunk_stream.next().await {
                if tx.send(chunk).await.is_err() {
                    break; // Receiver dropped
                }
            }
        });

        // Convert channel receiver to stream
        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed()
    }

    fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let credential = match self.credential_for_request() {
            Ok(value) => value.map(str::to_string),
            Err(err) => {
                return stream::once(async move { Err(StreamError::Provider(err.to_string())) })
                    .boxed();
            }
        };

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages = effective_messages
            .into_iter()
            .map(|message| Message {
                role: message.role,
                content: MessageContent::from_chat_text(&message.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(options.enabled),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        let client = self.http_client();
        let auth_header = self.auth_header.clone();
        let extra_headers = self.extra_headers.clone();
        let openrouter_attribution_headers = self.openrouter_attribution_headers();
        let provider_name = self.name.clone();
        let model_owned = model.to_string();

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

        tokio::spawn(async move {
            let mut req_builder = client.post(&url).json(&request);
            req_builder = match (&auth_header, credential.as_deref()) {
                (AuthStyle::None, _) | (_, None) => req_builder,
                (AuthStyle::Bearer, Some(credential)) => {
                    req_builder.header("Authorization", format!("Bearer {credential}"))
                }
                (AuthStyle::XApiKey, Some(credential)) => {
                    req_builder.header("x-api-key", credential)
                }
                (AuthStyle::Anthropic, Some(credential)) => req_builder
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01"),
                (AuthStyle::Custom(header), Some(credential)) => {
                    req_builder.header(header, credential)
                }
            };
            for (name, value) in &extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
            if let Some((referer, title)) = openrouter_attribution_headers {
                req_builder = req_builder
                    .header("HTTP-Referer", referer)
                    .header("X-OpenRouter-Title", title);
            }
            req_builder = req_builder.header("Accept", "text/event-stream");

            let response = match req_builder.send().await {
                Ok(response) => response,
                Err(error) => {
                    crate::core::observability::report_error(
                        error.to_string().as_str(),
                        "llm_provider",
                        "stream_chat_history",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("failure", "transport"),
                        ],
                    );
                    let _ = tx.send(Err(StreamError::Http(error))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let status_str = status.as_u16().to_string();
                let raw_error = match response.text().await {
                    Ok(error) => error,
                    Err(_) => format!("HTTP error: {status}"),
                };
                let sanitized_error = super::sanitize_api_error(&raw_error);
                let message = format!("{status}: {sanitized_error}");
                if super::is_budget_exhausted_http_400(status, &raw_error) {
                    super::log_budget_exhausted_http_400(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_custom_openai_upstream_bad_request_http_400(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    super::log_custom_openai_upstream_bad_request_http_400(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_provider_access_policy_denied_http_403(status, &raw_error) {
                    super::log_provider_access_policy_denied_http_403(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::is_provider_config_rejection_http(
                    status,
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    super::log_provider_config_rejection(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if super::should_report_provider_http_failure(status) {
                    crate::core::observability::report_error(
                        message.as_str(),
                        "llm_provider",
                        "stream_chat_history",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("status", status_str.as_str()),
                            ("failure", "non_2xx"),
                        ],
                    );
                }
                let _ = tx.send(Err(StreamError::Provider(message))).await;
                return;
            }

            let mut chunk_stream = sse_bytes_to_chunks(response, options.count_tokens);
            while let Some(chunk) = chunk_stream.next().await {
                if tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed()
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(credential) = self.credential.as_ref() {
            // Hit the chat completions URL with a GET to establish the connection pool.
            // The server will likely return 405 Method Not Allowed, which is fine -
            // the goal is TLS handshake and HTTP/2 negotiation.
            let url = self.chat_completions_url();
            let _ = self
                .apply_auth_header(self.http_client().get(&url), Some(credential.as_str()))
                .send()
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "compatible_tests.rs"]
mod tests;
