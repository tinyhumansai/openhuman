use crate::openhuman::tools::ToolSpec;
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::fmt::Write;

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default, skip_serializing)]
    pub id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing)]
    pub extra_metadata: Option<serde_json::Value>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "system".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "user".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "assistant".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "tool".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    /// Provider-specific passthrough metadata for this call, captured from the
    /// response and echoed back verbatim on the next assistant turn. Carries
    /// Google Gemini's required `extra_content.google.thought_signature` so
    /// multi-turn tool calling round-trips without a 400 (TAURI-RUST-4PK).
    /// `None`/omitted for every provider that doesn't emit it, so non-Gemini
    /// history stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<serde_json::Value>,
}

/// Token usage information returned by the provider after an inference call.
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
    /// Number of tokens in the input/prompt.
    pub input_tokens: u64,
    /// Number of tokens in the output/completion.
    pub output_tokens: u64,
    /// Total context window size for the model (0 if unknown).
    pub context_window: u64,
    /// Number of input tokens that were served from the KV cache
    /// (returned by backends that support prompt caching, e.g. via
    /// `openhuman.usage.cached_input_tokens` or
    /// `prompt_tokens_details.cached_tokens`).
    pub cached_input_tokens: u64,
    /// Amount billed for this request in USD (from
    /// `openhuman.billing.charged_amount_usd`). Zero when unavailable.
    pub charged_amount_usd: f64,
}

/// An LLM response that may contain text, tool calls, or both.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// Text content of the response (may be empty if only tool calls).
    pub text: Option<String>,
    /// Tool calls requested by the LLM.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage info from the provider (if available).
    pub usage: Option<UsageInfo>,
    /// Raw reasoning/thinking content returned by thinking models (e.g.
    /// DeepSeek-R1, Qwen3) in the `reasoning_content` field. This must be
    /// passed back verbatim on the next turn — the API returns HTTP 400
    /// ("reasoning_content in thinking mode must be passed back") if it is
    /// omitted from the assistant message in a multi-turn conversation.
    ///
    /// Stored separately from `text` so callers can preserve it through
    /// the conversation history without merging it into the visible reply.
    pub reasoning_content: Option<String>,
}

impl ChatResponse {
    /// True when the LLM wants to invoke at least one tool.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Convenience: return text content or empty string.
    pub fn text_or_empty(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

/// A fine-grained streaming event emitted by a provider while serving a
/// `chat()` call. Providers that support SSE/streaming forward these to
/// the optional sender on [`ChatRequest::stream`]; the final aggregated
/// response is still returned from `chat()` so callers that ignore the
/// stream keep working unchanged.
#[derive(Debug, Clone)]
pub enum ProviderDelta {
    /// A chunk of the assistant's visible text output.
    TextDelta { delta: String },
    /// A chunk of the model's reasoning/thinking output (for models
    /// that emit `reasoning_content` or an equivalent). Consumers should
    /// render this in a separate UI affordance from the visible output.
    ThinkingDelta { delta: String },
    /// The start of a new native tool call. `call_id` is the
    /// provider-assigned id that later appears on the result message.
    ToolCallStart { call_id: String, tool_name: String },
    /// A chunk of argument JSON text for an in-flight tool call.
    /// Streamed verbatim; may arrive as partial JSON that only becomes
    /// valid once the stream completes.
    ToolCallArgsDelta { call_id: String, delta: String },
}

/// Request payload for provider chat calls.
///
/// The system prompt is built once at session start and frozen for the
/// rest of the session — the inference backend's automatic prefix
/// cache covers the whole thing, so there is no explicit cache-boundary
/// to thread through the request.
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>,
    /// Optional sink for `ProviderDelta` events. When `Some`, providers
    /// that support streaming will ask the upstream API for SSE and
    /// forward fine-grained events here. Providers without a streaming
    /// implementation ignore the sender and return only the aggregated
    /// response.
    pub stream: Option<&'a tokio::sync::mpsc::Sender<ProviderDelta>>,
    /// Optional upper bound on output tokens to request from the provider
    /// (`max_tokens` on the OpenAI-compatible wire).
    ///
    /// Left `None` for open-ended generation (orchestrator, agent turns)
    /// where the model should use its full budget. Set to a small concrete
    /// value by callers whose output is bounded by construction — notably
    /// memory extraction, whose response is a tiny structured-JSON object.
    /// Beyond capping wasted generation, this stops credit-metered providers
    /// (e.g. OpenRouter) from reserving the model's *entire* output window
    /// during their pre-flight balance check: an unset `max_tokens` makes
    /// OpenRouter price the request against the full 64k+ window and 402 a
    /// low-balance BYO user who could easily afford the few thousand tokens
    /// an extraction actually needs (TAURI-RUST-C62).
    pub max_tokens: Option<u32>,
}

/// A tool result to feed back to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
}

/// A message in a multi-turn conversation, including tool interactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ConversationMessage {
    /// Regular chat message (system, user, assistant).
    Chat(ChatMessage),
    /// Tool calls from the assistant (stored for history fidelity).
    AssistantToolCalls {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    /// Results of tool executions, fed back to the LLM.
    ToolResults(Vec<ToolResultMessage>),
}

/// A chunk of content from a streaming response.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// Text delta for this chunk.
    pub delta: String,
    /// Whether this is the final chunk.
    pub is_final: bool,
    /// Approximate token count for this chunk (estimated).
    pub token_count: usize,
}

impl StreamChunk {
    /// Create a new non-final chunk.
    pub fn delta(text: impl Into<String>) -> Self {
        Self {
            delta: text.into(),
            is_final: false,
            token_count: 0,
        }
    }

    /// Create a final chunk.
    pub fn final_chunk() -> Self {
        Self {
            delta: String::new(),
            is_final: true,
            token_count: 0,
        }
    }

    /// Create an error chunk.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            delta: message.into(),
            is_final: true,
            token_count: 0,
        }
    }

    /// Estimate tokens (rough approximation: ~4 chars per token).
    pub fn with_token_estimate(mut self) -> Self {
        self.token_count = self.delta.len().div_ceil(4);
        self
    }
}

/// Options for streaming chat requests.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamOptions {
    /// Whether to enable streaming (default: true).
    pub enabled: bool,
    /// Whether to include token counts in chunks.
    pub count_tokens: bool,
}

impl StreamOptions {
    /// Create new streaming options with enabled flag.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            count_tokens: false,
        }
    }

    /// Enable token counting.
    pub fn with_token_count(mut self) -> Self {
        self.count_tokens = true;
        self
    }
}

/// Result type for streaming operations.
pub type StreamResult<T> = std::result::Result<T, StreamError>;

/// Errors that can occur during streaming.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("HTTP error: {0}")]
    Http(reqwest::Error),

    #[error("JSON parse error: {0}")]
    Json(serde_json::Error),

    #[error("Invalid SSE format: {0}")]
    InvalidSse(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Structured error returned when a requested capability is not supported.
#[derive(Debug, Clone, thiserror::Error)]
#[error("provider_capability_error provider={provider} capability={capability} message={message}")]
pub struct ProviderCapabilityError {
    pub provider: String,
    pub capability: String,
    pub message: String,
}

/// Provider capabilities declaration.
///
/// Describes what features a provider supports, enabling intelligent
/// adaptation of tool calling modes and request formatting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// Whether the provider supports native tool calling via API primitives.
    ///
    /// When `true`, the provider can convert tool definitions to API-native
    /// formats (e.g., Gemini's functionDeclarations, Anthropic's input_schema).
    ///
    /// When `false`, tools must be injected via system prompt as text.
    pub native_tool_calling: bool,
    /// Whether the provider supports vision / image inputs.
    pub vision: bool,
}

/// Prompt / KV-cache behaviour a provider supports.
///
/// Sibling to [`ProviderCapabilities`], surfaced via
/// [`Provider::prompt_cache_capabilities`] so the agent and cost layers can
/// pick a stable cache-key strategy and calibrate cached-token telemetry per
/// provider. Every field defaults to `false` (conservative): an unknown or
/// custom OpenAI-compatible provider is assumed to support no caching, so we
/// never infer cache behaviour — or send cache-only request fields — that the
/// upstream may not honour (#3939).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptCacheCapabilities {
    /// Provider transparently caches identical request prefixes server-side
    /// with no client action — e.g. OpenAI / DeepSeek / Anthropic implicit
    /// caching. A byte-stable prompt prefix then earns cache hits for free, so
    /// preserving the prefix is worthwhile for this provider.
    pub automatic_prefix_cache: bool,
    /// Provider accepts explicit cache-control / cache-boundary markers in the
    /// request body (e.g. Anthropic `cache_control`). OpenAI-compatible chat
    /// APIs do not, so this stays `false` for them — we must not send such
    /// fields to a provider that would reject or ignore them.
    pub explicit_cache_control: bool,
    /// Provider returns cached-input-token counts in its usage block
    /// (`prompt_tokens_details.cached_tokens` or
    /// `openhuman.usage.cached_input_tokens`), so [`UsageInfo::cached_input_tokens`]
    /// is populated and cached-prefix cost accounting is exact rather than
    /// estimated.
    pub usage_reports_cached_input: bool,
    /// Provider supports grouping calls by a stable logical key (thread /
    /// session) for cache locality — today only the OpenHuman backend, via its
    /// `thread_id` extension. Third-party providers rely on prefix identity
    /// instead and must not receive OpenHuman-only grouping fields.
    pub cache_key_grouping: bool,
}

/// Provider-specific tool payload formats.
///
/// Different LLM providers require different formats for tool definitions.
/// This enum encapsulates those variations, enabling providers to convert
/// from the unified `ToolSpec` format to their native API requirements.
#[derive(Debug, Clone)]
pub enum ToolsPayload {
    /// Gemini API format (functionDeclarations).
    Gemini {
        function_declarations: Vec<serde_json::Value>,
    },
    /// Anthropic Messages API format (tools with input_schema).
    Anthropic { tools: Vec<serde_json::Value> },
    /// OpenAI Chat Completions API format (tools with function).
    OpenAI { tools: Vec<serde_json::Value> },
    /// Prompt-guided fallback (tools injected as text in system prompt).
    PromptGuided { instructions: String },
}

fn should_log_prompts() -> bool {
    matches!(
        std::env::var("OPENHUMAN_LOG_PROMPTS").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn format_prompt_messages(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for (idx, msg) in messages.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let _ = writeln!(&mut out, "[{idx}] role={}", msg.role);
        out.push_str(&msg.content);
        out.push('\n');
    }
    out
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Query provider capabilities.
    ///
    /// Default implementation returns minimal capabilities (no native tool calling).
    /// Providers should override this to declare their actual capabilities.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Declare the provider's prompt / KV-cache behaviour.
    ///
    /// Default is the conservative all-`false` [`PromptCacheCapabilities`]:
    /// callers must not assume any caching for a provider that hasn't opted in.
    /// Providers that cache prefixes server-side, report cached input tokens,
    /// or support thread/session grouping override this to advertise it so the
    /// agent + cost layers get accurate cache telemetry and a stable cache-key
    /// strategy without leaking OpenHuman internals to providers that don't
    /// need them (#3939).
    fn prompt_cache_capabilities(&self) -> PromptCacheCapabilities {
        PromptCacheCapabilities::default()
    }

    /// Convert tool specifications to provider-native format.
    ///
    /// Default implementation returns `PromptGuided` payload, which injects
    /// tool documentation into the system prompt as text. Providers with
    /// native tool calling support should override this to return their
    /// specific format (Gemini, Anthropic, OpenAI).
    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
        ToolsPayload::PromptGuided {
            instructions: build_tool_instructions_text(tools),
        }
    }

    /// Simple one-shot chat (single user message, no explicit system prompt).
    ///
    /// This is the preferred API for non-agentic direct interactions.
    async fn simple_chat(
        &self,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        self.chat_with_system(None, message, model, temperature)
            .await
    }

    /// One-shot chat with optional system prompt.
    ///
    /// Kept for compatibility and advanced one-shot prompting.
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String>;

    /// Multi-turn conversation. Default implementation extracts the last user
    /// message and delegates to `chat_with_system`.
    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let last_user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.chat_with_system(system, last_user, model, temperature)
            .await
    }

    /// Structured chat API for agent loop callers.
    ///
    /// **`max_tokens` caveat:** the default implementation delegates to
    /// [`Self::chat_with_history`], whose signature carries no output-token
    /// budget, so a `request.max_tokens` set by the caller is **not** honored
    /// on this path. Providers that need to enforce an output cap (e.g. the
    /// OpenAI-compatible provider, which threads it onto the wire for
    /// credit-metered backends — TAURI-RUST-C62) override `chat()` directly.
    /// The drop is logged below rather than silently swallowed; it is not a
    /// hard error because no production caller both sets `max_tokens` and
    /// routes through a default-`chat()` provider (agent turns pass `None`;
    /// memory extraction uses the compatible provider).
    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        if let Some(cap) = request.max_tokens {
            log::debug!(
                "[provider] default chat() for model={model} ignores max_tokens={cap} — \
                 this provider does not override chat() and chat_with_history() carries no \
                 output budget; the cap will not reach the wire"
            );
        }
        let log_prompts = should_log_prompts();
        // If tools are provided but provider doesn't support native tools,
        // inject tool instructions into system prompt as fallback.
        if let Some(tools) = request.tools {
            if !tools.is_empty() && !self.supports_native_tools() {
                let tool_instructions = match self.convert_tools(tools) {
                    ToolsPayload::PromptGuided { instructions } => instructions,
                    payload => {
                        anyhow::bail!(
                            "Provider returned non-prompt-guided tools payload ({payload:?}) while supports_native_tools() is false"
                        )
                    }
                };
                let mut modified_messages = request.messages.to_vec();

                // Inject tool instructions into an existing system message.
                // If none exists, prepend one to the conversation.
                if let Some(system_message) =
                    modified_messages.iter_mut().find(|m| m.role == "system")
                {
                    if !system_message.content.is_empty() {
                        system_message.content.push_str("\n\n");
                    }
                    system_message.content.push_str(&tool_instructions);
                } else {
                    modified_messages.insert(0, ChatMessage::system(tool_instructions));
                }

                if log_prompts {
                    log::info!(
                        "[prompt] model={model}\n{}",
                        format_prompt_messages(&modified_messages)
                    );
                }

                let text = self
                    .chat_with_history(&modified_messages, model, temperature)
                    .await?;
                return Ok(ChatResponse {
                    text: Some(text),
                    tool_calls: Vec::new(),
                    usage: None,
                    reasoning_content: None,
                });
            }
        }

        if log_prompts {
            log::info!(
                "[prompt] model={model}\n{}",
                format_prompt_messages(request.messages)
            );
        }

        let text = self
            .chat_with_history(request.messages, model, temperature)
            .await?;
        Ok(ChatResponse {
            text: Some(text),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        })
    }

    /// Whether provider supports native tool calls over API.
    fn supports_native_tools(&self) -> bool {
        self.capabilities().native_tool_calling
    }

    /// Whether provider supports multimodal vision input.
    fn supports_vision(&self) -> bool {
        self.capabilities().vision
    }

    /// Effective context window (in tokens) for `model`, used for
    /// pre-dispatch history trimming.
    ///
    /// Defaults to the static model table
    /// ([`crate::openhuman::inference::context_window_for_model`]), which
    /// reflects a model's *trained maximum* context. Local providers
    /// override this to report the model's **runtime-loaded** window — e.g.
    /// LM Studio lets the user load a model with a smaller `n_ctx` than its
    /// trained maximum, and budgeting against the max overflows the loaded
    /// window so the request is rejected (issue #3550 / Sentry
    /// TAURI-RUST-6V0). `None` means "unknown — skip pre-dispatch trimming".
    async fn effective_context_window(&self, model: &str) -> Option<u64> {
        crate::openhuman::inference::context_window_for_model(model)
    }

    /// Whether this provider talks to a **local** runtime (LM Studio, Ollama,
    /// llama.cpp, vLLM, …) rather than a cloud API. Local runtimes enforce the
    /// model's *runtime-loaded* `n_ctx` and can be loaded with a window smaller
    /// than the assistant's un-evictable system prefix — the
    /// `n_keep >= n_ctx` overflow (#3550 / TAURI-RUST-6V0). The agent engine
    /// uses this to gate its pre-dispatch un-evictable-prefix guard, which
    /// surfaces an actionable "reload with a larger context length" error only
    /// for local providers (cloud windows are large enough that the guard would
    /// only ever fire on a genuine overflow the user can't remedy by reloading).
    /// Defaults to `false`.
    fn is_local_provider(&self) -> bool {
        false
    }

    /// Like [`Provider::is_local_provider`] but resolved for the specific
    /// `model` about to be dispatched. A router whose *default* provider is
    /// cloud may still route a given model to a local provider; the engine's
    /// pre-dispatch un-evictable-prefix guard keys off this so the actionable
    /// "reload with a larger context length" error fires for that routed local
    /// model instead of letting the opaque local `400 (n_keep >= n_ctx)` reach
    /// the user (#3550 / TAURI-RUST-6V0; Codex/CodeRabbit review on PR #3771).
    ///
    /// Defaults to the model-blind [`Provider::is_local_provider`]; only a
    /// routing wrapper needs to override it.
    fn is_local_provider_for_model(&self, _model: &str) -> bool {
        self.is_local_provider()
    }

    /// The model's **authoritative runtime-loaded** context window, when the
    /// local runtime actually reports it (e.g. LM Studio's native
    /// `/api/v0/models` `loaded_context_length`). Returns `None` whenever the
    /// window is unknown or merely *guessed* — a cloud provider, a local
    /// runtime that exposes no loaded window (llama.cpp / vLLM), or a
    /// profile-default / conservative-floor fallback.
    ///
    /// Distinct from [`Provider::effective_context_window`], which always
    /// yields a value for local providers (falling back to a guess) so
    /// pre-dispatch *trimming* still engages. Trimming may safely run against a
    /// guess (over-trim is harmless), but the hard pre-dispatch abort must only
    /// fire on an authoritative window — aborting with "reload with a larger
    /// context length" against a guessed 4096 floor would wrongly reject a
    /// request that the real (e.g. 32k) loaded window would have accepted
    /// (Codex P1 review on PR #3771). Defaults to `None`.
    async fn loaded_context_window(&self, _model: &str) -> Option<u64> {
        None
    }

    /// Warm up the HTTP connection pool (TLS handshake, DNS, HTTP/2 setup).
    /// Default implementation is a no-op; providers with HTTP clients should override.
    async fn warmup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Chat with tool definitions for native function calling support.
    /// The default implementation falls back to chat_with_history and returns
    /// an empty tool_calls vector (prompt-based tool use only).
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let text = self.chat_with_history(messages, model, temperature).await?;
        Ok(ChatResponse {
            text: Some(text),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        })
    }

    /// Whether provider supports streaming responses.
    /// Default implementation returns false.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Streaming chat with optional system prompt.
    /// Returns an async stream of text chunks.
    /// Default implementation falls back to non-streaming chat.
    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        // Default: return an empty stream (not supported)
        stream::empty().boxed()
    }

    /// Streaming chat with history.
    /// Default implementation falls back to stream_chat_with_system with last user message.
    fn stream_chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        // For default implementation, we need to convert to owned strings
        // This is a limitation of the default implementation
        let provider_name = "unknown".to_string();

        // Create a single empty chunk to indicate not supported
        let chunk = StreamChunk::error(format!("{} does not support streaming", provider_name));
        stream::once(async move { Ok(chunk) }).boxed()
    }
}

/// Build tool instructions text for prompt-guided tool calling.
///
/// Generates a formatted text block describing available tools and how to
/// invoke them using XML-style tags. This is used as a fallback when the
/// provider doesn't support native tool calling.
pub fn build_tool_instructions_text(tools: &[ToolSpec]) -> String {
    let mut instructions = String::new();

    instructions.push_str("## Tool Use Protocol\n\n");
    instructions.push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
    instructions.push_str("<tool_call>\n");
    instructions.push_str(r#"{"name": "tool_name", "arguments": {"param": "value"}}"#);
    instructions.push_str("\n</tool_call>\n\n");
    instructions.push_str("You may use multiple tool calls in a single response. ");
    instructions.push_str("After tool execution, results appear in <tool_result> tags. ");
    instructions
        .push_str("Continue reasoning with the results until you can give a final answer.\n\n");
    instructions.push_str("### Available Tools\n\n");

    for tool in tools {
        writeln!(&mut instructions, "**{}**: {}", tool.name, tool.description)
            .expect("writing to String cannot fail");

        let parameters =
            serde_json::to_string(&tool.parameters).unwrap_or_else(|_| "{}".to_string());
        writeln!(&mut instructions, "Parameters: `{parameters}`")
            .expect("writing to String cannot fail");
        instructions.push('\n');
    }

    instructions
}

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
