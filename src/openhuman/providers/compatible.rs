//! Generic OpenAI-compatible provider.
//! Most LLM APIs follow the same `/v1/chat/completions` format.
//! This module provides a single implementation that works for all of them.

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
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic sequence so multiple requests in the same millisecond sort
/// deterministically in the dump directory.
static PROMPT_DUMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// When `OPENHUMAN_PROMPT_DUMP_DIR` is set, write `body` (the exact JSON
/// payload we're about to POST to the provider) to a timestamped file
/// under that directory. Best-effort: failures are logged and swallowed
/// so a dump outage never breaks inference.
///
/// Intended for KV-cache debugging — diff consecutive turns to see which
/// bytes of the prefix drifted and broke the cache hit.
/// Write raw response bytes to the dump dir paired with the most-recent
/// prompt file (same `seq` prefix, `.response.json` suffix). `seq` must
/// be the value reserved via `reserve_dump_seq` and passed to
/// `dump_prompt_if_enabled` so request/response files sort next to
/// each other.
fn dump_response_if_enabled(provider: &str, model: &str, seq: u64, bytes: &[u8]) {
    let Ok(dir) = std::env::var("OPENHUMAN_PROMPT_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "[prompt_dump] failed to create dir {}: {err}",
            dir.display()
        );
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let safe_model: String = model
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("{ts}_{seq:06}_{provider}_{safe_model}.response.json");
    let path = dir.join(filename);
    // Re-pretty-print if it parses as JSON so diffs are stable; otherwise
    // write raw bytes (SSE fragments, error HTML, etc).
    let payload: Vec<u8> = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => serde_json::to_vec_pretty(&v).unwrap_or_else(|_| bytes.to_vec()),
        Err(_) => bytes.to_vec(),
    };
    if let Err(err) = std::fs::write(&path, &payload) {
        log::warn!(
            "[prompt_dump] response write failed {}: {err}",
            path.display()
        );
    } else {
        log::debug!(
            "[prompt_dump] wrote response {} bytes -> {}",
            payload.len(),
            path.display()
        );
    }
}

/// Atomically reserve the next dump sequence number. This is the single
/// source of truth for seq allocation — both the prompt dump and its
/// paired response dump must use the value returned here. A non-atomic
/// peek-then-increment split would race under concurrent requests (two
/// callers could reserve the same seq or correlate a request/response
/// pair across different turns).
fn reserve_dump_seq() -> u64 {
    PROMPT_DUMP_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn dump_prompt_if_enabled<T: Serialize>(provider: &str, model: &str, seq: u64, body: &T) {
    let Ok(dir) = std::env::var("OPENHUMAN_PROMPT_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!(
            "[prompt_dump] failed to create dir {}: {err}",
            dir.display()
        );
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let safe_model: String = model
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("{ts}_{seq:06}_{provider}_{safe_model}.json");
    let path = dir.join(filename);
    match serde_json::to_vec_pretty(body) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(&path, &bytes) {
                log::warn!("[prompt_dump] write failed {}: {err}", path.display());
            } else {
                log::debug!(
                    "[prompt_dump] wrote {} bytes -> {}",
                    bytes.len(),
                    path.display()
                );
            }
        }
        Err(err) => {
            log::warn!("[prompt_dump] serialize failed: {err}");
        }
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
    user_agent: Option<String>,
    /// When true, collect all `system` messages and prepend their content
    /// to the first `user` message, then drop the system messages.
    /// Required for providers that reject `role: system` (e.g. MiniMax).
    merge_system_into_user: bool,
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
}

#[derive(Debug, Serialize)]
struct ApiChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiChatResponse {
    choices: Vec<Choice>,
    /// Standard OpenAI usage block.
    #[serde(default)]
    usage: Option<ApiUsage>,
    /// OpenHuman backend metadata (usage + billing summary).
    #[serde(default)]
    openhuman: Option<OpenHumanMeta>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

/// Standard OpenAI `usage` block on a chat completion response.
#[derive(Debug, Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize, Default)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

/// OpenHuman backend metadata appended to the response JSON.
#[derive(Debug, Deserialize, Default)]
struct OpenHumanMeta {
    #[serde(default)]
    usage: Option<OpenHumanUsage>,
    #[serde(default)]
    billing: Option<OpenHumanBilling>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenHumanUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    #[allow(dead_code)]
    total_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenHumanBilling {
    #[serde(default)]
    charged_amount_usd: f64,
}

/// Remove `<think>...</think>` blocks from model output.
/// Some reasoning models (e.g. MiniMax) embed their chain-of-thought inline
/// in the `content` field rather than a separate `reasoning_content` field.
/// The resulting `<think>` tags must be stripped before returning to the user.
fn strip_think_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                // Unclosed tag: drop the rest to avoid leaking partial reasoning.
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    /// Reasoning/thinking models (e.g. Qwen3, GLM-4) may return their output
    /// in `reasoning_content` instead of `content`. Used as automatic fallback.
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    function_call: Option<Function>,
}

impl ResponseMessage {
    /// Extract text content, falling back to `reasoning_content` when `content`
    /// is missing or empty. Reasoning/thinking models (Qwen3, GLM-4, etc.)
    /// often return their output solely in `reasoning_content`.
    /// Strips `<think>...</think>` blocks that some models (e.g. MiniMax) embed
    /// inline in `content` instead of using a separate field.
    fn effective_content(&self) -> String {
        if let Some(content) = self.content.as_ref().filter(|c| !c.is_empty()) {
            let stripped = strip_think_tags(content);
            if !stripped.is_empty() {
                return stripped;
            }
        }

        self.reasoning_content
            .as_ref()
            .map(|c| strip_think_tags(c))
            .filter(|c| !c.is_empty())
            .unwrap_or_default()
    }

    fn effective_content_optional(&self) -> Option<String> {
        if let Some(content) = self.content.as_ref().filter(|c| !c.is_empty()) {
            let stripped = strip_think_tags(content);
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }

        self.reasoning_content
            .as_ref()
            .map(|c| strip_think_tags(c))
            .filter(|c| !c.is_empty())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<Function>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Function {
    name: Option<String>,
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct NativeChatRequest {
    model: String,
    messages: Vec<NativeMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
struct NativeMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<ResponsesInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ResponsesInput {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    #[serde(default)]
    output: Vec<ResponsesOutput>,
    #[serde(default)]
    output_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutput {
    #[serde(default)]
    content: Vec<ResponsesContent>,
}

#[derive(Debug, Deserialize)]
struct ResponsesContent {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

// ---------------------------------------------------------------
// Streaming support (SSE parser)
// ---------------------------------------------------------------

/// Server-Sent Event stream chunk for OpenAI-compatible streaming.
#[derive(Debug, Deserialize)]
struct StreamChunkResponse {
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[serde(default)]
    openhuman: Option<OpenHumanMeta>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    /// Reasoning/thinking models may stream output via `reasoning_content`.
    #[serde(default)]
    reasoning_content: Option<String>,
    /// Native tool-call chunks. Each entry is keyed by `index`; the first
    /// chunk for a given index carries `id`/`type`/`function.name`, later
    /// chunks only carry fragments of `function.arguments`.
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    /// Index of this tool call within the assistant message. Multiple
    /// concurrent tool calls share the same message and are distinguished
    /// by index — not id (which may only appear on the first chunk).
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    #[allow(dead_code)]
    kind: Option<String>,
    #[serde(default)]
    function: Option<StreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallFunction {
    #[serde(default)]
    name: Option<String>,
    /// Arguments are streamed as a raw JSON string fragment; we accumulate
    /// them as-is and only parse at the end of the stream.
    #[serde(default)]
    arguments: Option<String>,
}

/// Parse SSE (Server-Sent Events) stream from OpenAI-compatible providers.
/// Handles the `data: {...}` format and `[DONE]` sentinel.
fn parse_sse_line(line: &str) -> StreamResult<Option<String>> {
    let line = line.trim();

    // Skip empty lines and comments
    if line.is_empty() || line.starts_with(':') {
        return Ok(None);
    }

    // SSE format: "data: {...}"
    if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim();

        // Check for [DONE] sentinel
        if data == "[DONE]" {
            return Ok(None);
        }

        // Parse JSON delta
        let chunk: StreamChunkResponse = serde_json::from_str(data).map_err(StreamError::Json)?;

        // Extract content from delta
        if let Some(choice) = chunk.choices.first() {
            if let Some(content) = &choice.delta.content {
                if !content.is_empty() {
                    return Ok(Some(content.clone()));
                }
            }
            // Fallback to reasoning_content for thinking models
            if let Some(reasoning) = &choice.delta.reasoning_content {
                return Ok(Some(reasoning.clone()));
            }
        }
    }

    Ok(None)
}

/// Convert SSE byte stream to text chunks.
fn sse_bytes_to_chunks(
    response: reqwest::Response,
    count_tokens: bool,
) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
    // Create a channel to send chunks
    let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

    tokio::spawn(async move {
        // Buffer for incomplete lines
        let mut buffer = String::new();

        // Get response body as bytes stream
        match response.error_for_status_ref() {
            Ok(_) => {}
            Err(e) => {
                let _ = tx.send(Err(StreamError::Http(e))).await;
                return;
            }
        }

        let mut bytes_stream = response.bytes_stream();

        while let Some(item) = bytes_stream.next().await {
            match item {
                Ok(bytes) => {
                    // Convert bytes to string and process line by line
                    let text = match String::from_utf8(bytes.to_vec()) {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(Err(StreamError::InvalidSse(format!(
                                    "Invalid UTF-8: {}",
                                    e
                                ))))
                                .await;
                            break;
                        }
                    };

                    buffer.push_str(&text);

                    // Process complete lines
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer.drain(..=pos).collect::<String>();
                        buffer = buffer[pos + 1..].to_string();

                        match parse_sse_line(&line) {
                            Ok(Some(content)) => {
                                let mut chunk = StreamChunk::delta(content);
                                if count_tokens {
                                    chunk = chunk.with_token_estimate();
                                }
                                if tx.send(Ok(chunk)).await.is_err() {
                                    return; // Receiver dropped
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let _ = tx.send(Err(e)).await;
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(StreamError::Http(e))).await;
                    break;
                }
            }
        }

        // Send final chunk
        let _ = tx.send(Ok(StreamChunk::final_chunk())).await;
    });

    // Convert channel receiver to stream
    stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|chunk| (chunk, rx))
    })
    .boxed()
}

fn first_nonempty(text: Option<&str>) -> Option<String> {
    text.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_responses_role(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        "tool" => "assistant",
        _ => "user",
    }
}

fn build_responses_prompt(messages: &[ChatMessage]) -> (Option<String>, Vec<ResponsesInput>) {
    let mut instructions_parts = Vec::new();
    let mut input = Vec::new();

    for message in messages {
        if message.content.trim().is_empty() {
            continue;
        }

        if message.role == "system" {
            instructions_parts.push(message.content.clone());
            continue;
        }

        input.push(ResponsesInput {
            role: normalize_responses_role(&message.role).to_string(),
            content: message.content.clone(),
        });
    }

    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n\n"))
    };

    (instructions, input)
}

fn extract_responses_text(response: ResponsesResponse) -> Option<String> {
    if let Some(text) = first_nonempty(response.output_text.as_deref()) {
        return Some(text);
    }

    for item in &response.output {
        for content in &item.content {
            if content.kind.as_deref() == Some("output_text") {
                if let Some(text) = first_nonempty(content.text.as_deref()) {
                    return Some(text);
                }
            }
        }
    }

    for item in &response.output {
        for content in &item.content {
            if let Some(text) = first_nonempty(content.text.as_deref()) {
                return Some(text);
            }
        }
    }

    None
}

fn compact_sanitized_body_snippet(body: &str) -> String {
    super::sanitize_api_error(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_chat_response_body(provider_name: &str, body: &str) -> anyhow::Result<ApiChatResponse> {
    serde_json::from_str::<ApiChatResponse>(body).map_err(|error| {
        let snippet = compact_sanitized_body_snippet(body);
        anyhow::anyhow!(
            "{provider_name} API returned an unexpected chat-completions payload: {error}; body={snippet}"
        )
    })
}

fn normalize_function_arguments(arguments: Option<serde_json::Value>) -> String {
    match arguments {
        Some(serde_json::Value::String(raw)) => {
            if raw.trim().is_empty() {
                "{}".to_string()
            } else {
                raw
            }
        }
        Some(serde_json::Value::Null) | None => "{}".to_string(),
        Some(other) => serde_json::to_string(&other).unwrap_or_else(|_| "{}".to_string()),
    }
}

fn parse_provider_tool_call_from_value(value: &serde_json::Value) -> Option<ProviderToolCall> {
    if let Ok(call) = serde_json::from_value::<ProviderToolCall>(value.clone()) {
        if !call.name.trim().is_empty() {
            return Some(ProviderToolCall {
                id: if call.id.trim().is_empty() {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    call.id
                },
                name: call.name,
                arguments: if call.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    call.arguments
                },
            });
        }
    }

    let function = value.get("function")?;
    let name = function.get("name").and_then(serde_json::Value::as_str)?;
    if name.trim().is_empty() {
        return None;
    }

    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments: normalize_function_arguments(function.get("arguments").cloned()),
    })
}

fn parse_tool_calls_from_content_json(
    content: &str,
) -> Option<(Option<String>, Vec<ProviderToolCall>)> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let tool_calls_value = value.get("tool_calls")?.as_array()?;
    let tool_calls: Vec<ProviderToolCall> = tool_calls_value
        .iter()
        .filter_map(parse_provider_tool_call_from_value)
        .collect();
    if tool_calls.is_empty() {
        return None;
    }

    let text = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    Some((text, tool_calls))
}

fn parse_responses_response_body(
    provider_name: &str,
    body: &str,
) -> anyhow::Result<ResponsesResponse> {
    serde_json::from_str::<ResponsesResponse>(body).map_err(|error| {
        let snippet = compact_sanitized_body_snippet(body);
        anyhow::anyhow!(
            "{provider_name} Responses API returned an unexpected payload: {error}; body={snippet}"
        )
    })
}

impl OpenAiCompatibleProvider {
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
                                            arguments: Some(serde_json::Value::String(tc.arguments)),
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
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content) {
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

/// Per-index tool-call accumulator used while consuming an SSE stream.
///
/// `arguments` holds the full cumulative JSON text fragments seen so
/// far. `emitted_start` tracks whether we've surfaced the synthetic
/// `ProviderDelta::ToolCallStart` event yet (we only do once we know
/// both `id` and `name`). `emitted_chars` is the byte offset within
/// `arguments` that we've already flushed as `ToolCallArgsDelta`
/// events — used to avoid re-sending buffered fragments after the
/// start event fires.
#[derive(Debug, Default)]
struct StreamingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    emitted_start: bool,
    emitted_chars: usize,
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

        let native_request = NativeChatRequest {
            model: model.to_string(),
            messages: Self::convert_messages_for_native(&effective_messages),
            temperature,
            stream: Some(false),
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
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
