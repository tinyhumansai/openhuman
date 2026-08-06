use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::tools::ToolSpec;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

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
    /// Number of input tokens written into a provider prompt/KV cache on this
    /// request (cache-creation / cache-write tokens). Distinct from
    /// `cached_input_tokens` (cache reads). Zero when the provider does not
    /// report a cache-write breakdown.
    pub cache_creation_tokens: u64,
    /// Number of reasoning/thinking output tokens when the provider exposes
    /// them separately from `output_tokens`. Zero when unavailable.
    pub reasoning_tokens: u64,
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

/// Upper bound on output tokens requested for an agent chat turn.
///
/// The agent loop used to leave `ChatRequest::max_tokens` `None` ("open-ended
/// generation"), but an unset cap makes reservation-pricing providers (e.g.
/// OpenRouter) reserve credit against the model's *entire* output window
/// (64k+) during their pre-flight balance check — so a modest-balance BYO user
/// can hit a `402` purely from the oversized reservation, a **preventable**
/// condition. Capping every agent turn at a realistic ceiling prices the
/// pre-flight against a budget the user can actually afford; a residual `402`
/// is then the genuine flat-balance case the insufficient-credits demote arm
/// is meant for (TAURI-RUST-C62; mirrors [`EXTRACTION_MAX_OUTPUT_TOKENS`] in
/// `memory_tree::score::extract::llm`).
///
/// `16384` sits comfortably above any realistic single agent turn — `max_tokens`
/// is an upper bound, not a forced length, so the model still stops at its
/// natural end well below the cap on normal turns — while cutting the
/// reservation 4× versus a 64k window.
pub const AGENT_TURN_MAX_OUTPUT_TOKENS: u32 = 16384;

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
    /// Left `None` only for the orchestrator's open-ended generation. Agent
    /// turns cap at [`AGENT_TURN_MAX_OUTPUT_TOKENS`] and callers whose output
    /// is bounded by construction set a small concrete value — notably memory
    /// extraction, whose response is a tiny structured-JSON object.
    /// Beyond capping wasted generation, this stops credit-metered providers
    /// (e.g. OpenRouter) from reserving the model's *entire* output window
    /// during their pre-flight balance check: an unset `max_tokens` makes
    /// OpenRouter price the request against the full 64k+ window and 402 a
    /// low-balance BYO user who could easily afford the few thousand tokens
    /// the turn actually needs (TAURI-RUST-C62).
    pub max_tokens: Option<u32>,
}

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
