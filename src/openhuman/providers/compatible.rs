//! Generic OpenAI-compatible provider.
//! Most LLM APIs follow the same `/v1/chat/completions` format.
//! This module provides a single implementation that works for all of them.

#[path = "compatible_dump.rs"]
mod compatible_dump;
#[path = "compatible_parse.rs"]
mod compatible_parse;
#[path = "compatible_stream.rs"]
mod compatible_stream;
#[path = "compatible_types.rs"]
mod compatible_types;

pub(crate) use compatible_parse::{parse_sse_line, strip_think_tags};
pub(crate) use compatible_types::ResponsesResponse;

use crate::openhuman::providers::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, StreamChunk, StreamError, StreamOptions, StreamResult, ToolCall as ProviderToolCall,
    UsageInfo as ProviderUsageInfo,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use reqwest::{
    header::{HeaderMap, HeaderValue, USER_AGENT},
    Client,
};

use compatible_dump::{dump_prompt_if_enabled, dump_response_if_enabled, reserve_dump_seq};
use compatible_parse::{
    build_responses_prompt, extract_responses_text, normalize_function_arguments,
    parse_chat_response_body, parse_responses_response_body, parse_tool_calls_from_content_json,
};
use compatible_stream::sse_bytes_to_chunks;
use compatible_types::{
    ApiChatRequest, ApiChatResponse, ApiUsage, Choice, Function, Message, NativeChatRequest,
    NativeMessage, OpenAiStreamOptions, OpenHumanMeta, ResponseMessage, ResponsesRequest,
    StreamChunkResponse, StreamingToolCall, ToolCall,
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
    user_agent: Option<String>,
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
}

/// How the provider expects the API key to be sent.
#[derive(Debug, Clone)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>`
    Bearer,
    /// `x-api-key: <key>` (used by some Chinese providers)
    XApiKey,
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
            user_agent: user_agent.map(ToString::to_string),
            merge_system_into_user,
            emit_openhuman_thread_id: false,
        }
    }

    /// Read the ambient `thread_id` only when this provider has been
    /// opted in via [`with_openhuman_thread_id`]. Returns `None` for
    /// every third-party provider so the field is omitted by
    /// `skip_serializing_if`.
    fn outbound_thread_id(&self) -> Option<String> {
        if self.emit_openhuman_thread_id {
            super::thread_context::current_thread_id()
        } else {
            None
        }
    }

    /// Collect all `system` role messages, concatenate their content,
    /// and prepend to the first `user` message. Drop all system messages.
    /// Used for providers (e.g. MiniMax) that reject `role: system`.
    fn flatten_system_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
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

    fn http_client(&self) -> Client {
        if let Some(ua) = self.user_agent.as_deref() {
            let mut headers = HeaderMap::new();
            if let Ok(value) = HeaderValue::from_str(ua) {
                headers.insert(USER_AGENT, value);
            }

            let builder = Client::builder()
                .use_rustls_tls()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(10))
                .default_headers(headers);
            let builder = crate::openhuman::config::apply_runtime_proxy_to_builder(
                builder,
                "provider.compatible",
            );

            return builder.build().unwrap_or_else(|error| {
                tracing::warn!("Failed to build proxied timeout client with user-agent: {error}");
                Client::new()
            });
        }

        let builder = Client::builder()
            .use_rustls_tls()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10));
        let builder = crate::openhuman::config::apply_runtime_proxy_to_builder(
            builder,
            "provider.compatible",
        );
        builder.build().unwrap_or_else(|error| {
            tracing::warn!("Failed to build proxied timeout client: {error}");
            Client::new()
        })
    }

    /// Build the full URL for chat completions, detecting if base_url already includes the path.
    /// This allows custom providers with non-standard endpoints (e.g., VolcEngine ARK uses
    /// `/api/coding/v3/chat/completions` instead of `/v1/chat/completions`).
    fn chat_completions_url(&self) -> String {
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

        if has_full_endpoint {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }

    fn path_ends_with(&self, suffix: &str) -> bool {
        if let Ok(url) = reqwest::Url::parse(&self.base_url) {
            return url.path().trim_end_matches('/').ends_with(suffix);
        }

        self.base_url.trim_end_matches('/').ends_with(suffix)
    }

    fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };

        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    /// Build the full URL for responses API, detecting if base_url already includes the path.
    fn responses_url(&self) -> String {
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

    fn tool_specs_to_openai_format(
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

    fn apply_auth_header(
        &self,
        req: reqwest::RequestBuilder,
        credential: &str,
    ) -> reqwest::RequestBuilder {
        match &self.auth_header {
            AuthStyle::Bearer => req.header("Authorization", format!("Bearer {credential}")),
            AuthStyle::XApiKey => req.header("x-api-key", credential),
            AuthStyle::Custom(header) => req.header(header, credential),
        }
    }

    async fn chat_via_responses(
        &self,
        credential: &str,
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

        let request = ResponsesRequest {
            model: model.to_string(),
            input,
            instructions,
            stream: Some(false),
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            anyhow::bail!("{} Responses API error: {error}", self.name);
        }

        let body = response.text().await?;
        let responses = parse_responses_response_body(&self.name, &body)?;

        extract_responses_text(responses)
            .ok_or_else(|| anyhow::anyhow!("No response from {} Responses API", self.name))
    }

    fn convert_tool_specs(
        tools: Option<&[crate::openhuman::tools::ToolSpec]>,
    ) -> Option<Vec<serde_json::Value>> {
        tools.map(|items| {
            items
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect()
        })
    }

    fn convert_messages_for_native(messages: &[ChatMessage]) -> Vec<NativeMessage> {
        messages
            .iter()
            .map(|message| {
                if message.role == "assistant" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content)
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

                                let content = value
                                    .get("content")
                                    .and_then(serde_json::Value::as_str)
                                    .map(ToString::to_string);

                                return NativeMessage {
                                    role: "assistant".to_string(),
                                    content,
                                    tool_call_id: None,
                                    tool_calls: Some(tool_calls),
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
                            .or_else(|| Some(message.content.clone()));

                        return NativeMessage {
                            role: "tool".to_string(),
                            content,
                            tool_call_id,
                            tool_calls: None,
                        };
                    }
                }

                NativeMessage {
                    role: message.role.clone(),
                    content: Some(message.content.clone()),
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect()
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

        let instructions = crate::openhuman::providers::traits::build_tool_instructions_text(tools);
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

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
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
        credential: &str,
        native_request: &NativeChatRequest,
        delta_tx: &tokio::sync::mpsc::Sender<crate::openhuman::providers::ProviderDelta>,
        dump_seq: u64,
    ) -> anyhow::Result<ProviderChatResponse> {
        use futures_util::StreamExt;

        let url = self.chat_completions_url();
        log::debug!(
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
            let body = response.text().await.unwrap_or_default();
            // Sanitize the upstream error body so we don't leak user
            // prompts, tool arguments, or credentials the backend
            // echoed back into the anyhow chain / logs.
            let sanitized = super::sanitize_api_error(&body);
            anyhow::bail!(
                "{} streaming API error ({}): {}",
                self.name,
                status,
                sanitized
            );
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
            log::debug!(
                "[stream] {} upstream replied with non-SSE content-type; falling back to JSON parse",
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

        while let Some(item) = bytes_stream.next().await {
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
                                    .send(crate::openhuman::providers::ProviderDelta::TextDelta {
                                        delta: content.clone(),
                                    })
                                    .await;
                            }
                        }
                        // Reasoning / thinking delta.
                        if let Some(reasoning) = choice.delta.reasoning_content.as_ref() {
                            if !reasoning.is_empty() {
                                thinking_accum.push_str(reasoning);
                                let _ = delta_tx
                                    .send(
                                        crate::openhuman::providers::ProviderDelta::ThinkingDelta {
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
                                            .send(crate::openhuman::providers::ProviderDelta::ToolCallStart {
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
                                                .send(crate::openhuman::providers::ProviderDelta::ToolCallArgsDelta {
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
                                            .send(crate::openhuman::providers::ProviderDelta::ToolCallArgsDelta {
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
                        // `normalize_function_arguments` can handle the
                        // usual Value path; fall back to a JSON-string
                        // value if the accumulated text isn't valid
                        // JSON yet.
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
    fn capabilities(&self) -> crate::openhuman::providers::traits::ProviderCapabilities {
        crate::openhuman::providers::traits::ProviderCapabilities {
            native_tool_calling: true,
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
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} API key not set. Configure via the web UI or set the appropriate env var.",
                self.name
            )
        })?;

        let mut messages = Vec::new();

        if self.merge_system_into_user {
            let content = match system_prompt {
                Some(sys) => format!("{sys}\n\n{message}"),
                None => message.to_string(),
            };
            messages.push(Message {
                role: "user".to_string(),
                content,
            });
        } else {
            if let Some(sys) = system_prompt {
                messages.push(Message {
                    role: "system".to_string(),
                    content: sys.to_string(),
                });
            }
            messages.push(Message {
                role: "user".to_string(),
                content: message.to_string(),
            });
        }

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature,
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

            anyhow::bail!("{} API error ({status}): {sanitized}", self.name);
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
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} API key not set. Configure via the web UI or set the appropriate env var.",
                self.name
            )
        })?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
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

            // Mirror chat_with_system: 404 may mean this provider uses the Responses API
            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &effective_messages, model)
                    .await
                    .map_err(|responses_err| {
                        let fb = super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            return Err(super::api_error(&self.name, response).await);
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
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} API key not set. Configure via the web UI or set the appropriate env var.",
                self.name
            )
        })?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
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

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
        })
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} API key not set. Configure via the web UI or set the appropriate env var.",
                self.name
            )
        })?;

        let tools = Self::convert_tool_specs(request.tools);
        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(request.messages)
        } else {
            request.messages.to_vec()
        };

        // ── Streaming branch ─────────────────────────────────────────
        // When the caller supplied a `ProviderDelta` sender, request
        // SSE and forward fine-grained deltas while accumulating the
        // final response. Fall back to non-streaming on non-200 errors
        // so tool-schema rejections etc. still work.
        if let Some(tx) = request.stream {
            let native_request = NativeChatRequest {
                model: model.to_string(),
                messages: Self::convert_messages_for_native(&effective_messages),
                temperature,
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
            };
            let stream_dump_seq = reserve_dump_seq();
            dump_prompt_if_enabled(&self.name, model, stream_dump_seq, &native_request);
            match self
                .stream_native_chat(credential, &native_request, tx, stream_dump_seq)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    log::warn!(
                        "[stream] {} streaming chat failed, falling back to non-streaming: {}",
                        self.name,
                        err
                    );
                    // Fall through to the non-streaming path below.
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
            temperature,
            stream: Some(false),
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
            thread_id,
            stream_options: None,
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
                });
            }

            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &effective_messages, model)
                    .await
                    .map(|text| ProviderChatResponse {
                        text: Some(text),
                        tool_calls: vec![],
                        usage: None,
                    })
                    .map_err(|responses_err| {
                        let fb = super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            anyhow::bail!("{} API error ({status}): {sanitized}", self.name);
        }

        let response_bytes = response.bytes().await?;
        dump_response_if_enabled(&self.name, model, dump_seq, &response_bytes);
        let native_response: ApiChatResponse = serde_json::from_slice(&response_bytes)
            .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;
        Self::parse_native_response(native_response, &self.name)
    }

    fn supports_native_tools(&self) -> bool {
        true
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
        let credential = match self.credential.as_ref() {
            Some(value) => value.clone(),
            None => {
                let provider_name = self.name.clone();
                return stream::once(async move {
                    Err(StreamError::Provider(format!(
                        "{} API key not set",
                        provider_name
                    )))
                })
                .boxed();
            }
        };

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system".to_string(),
                content: sys.to_string(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: message.to_string(),
        });

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature,
            stream: Some(options.enabled),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        let client = self.http_client();
        let auth_header = self.auth_header.clone();

        // Use a channel to bridge the async HTTP response to the stream
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

        tokio::spawn(async move {
            // Build request with auth
            let mut req_builder = client.post(&url).json(&request);

            // Apply auth header
            req_builder = match &auth_header {
                AuthStyle::Bearer => {
                    req_builder.header("Authorization", format!("Bearer {}", credential))
                }
                AuthStyle::XApiKey => req_builder.header("x-api-key", &credential),
                AuthStyle::Custom(header) => req_builder.header(header, &credential),
            };

            // Set accept header for streaming
            req_builder = req_builder.header("Accept", "text/event-stream");

            // Send request
            let response = match req_builder.send().await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Err(StreamError::Http(e))).await;
                    return;
                }
            };

            // Check status
            if !response.status().is_success() {
                let status = response.status();
                let error = match response.text().await {
                    Ok(e) => e,
                    Err(_) => format!("HTTP error: {}", status),
                };
                let _ = tx
                    .send(Err(StreamError::Provider(format!("{}: {}", status, error))))
                    .await;
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

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(credential) = self.credential.as_ref() {
            // Hit the chat completions URL with a GET to establish the connection pool.
            // The server will likely return 405 Method Not Allowed, which is fine -
            // the goal is TLS handshake and HTTP/2 negotiation.
            let url = self.chat_completions_url();
            let _ = self
                .apply_auth_header(self.http_client().get(&url), credential)
                .send()
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "compatible_tests.rs"]
mod tests;
