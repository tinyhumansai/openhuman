//! Serde request/response structs for the OpenAI-compatible provider.
//!
//! All types in this module are crate-internal (`pub(crate)` or `pub(crate)`
//! as appropriate). External code only sees the public API on
//! [`super::OpenAiCompatibleProvider`].

use serde::{Deserialize, Serialize};

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ApiChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Message {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct NativeChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<NativeMessage>,
    pub(crate) temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<String>,
    /// OpenHuman backend extension: stable conversation identifier so the
    /// server can group `InferenceLog` entries and align KV-cache keys
    /// with the same logical chat thread the user sees in the UI. Skipped
    /// when serialising for vanilla OpenAI-compatible providers that
    /// don't recognise it (most reject only unknown *required* fields,
    /// but emitting it here is gated on the ambient task-local being
    /// set — see `crate::openhuman::providers::thread_context`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thread_id: Option<String>,
    /// OpenAI streaming `stream_options`. Set to `{"include_usage": true}`
    /// on streaming requests so the server emits a final usage chunk
    /// (carrying token counts and `openhuman.billing.charged_amount_usd`
    /// when the OpenHuman backend is in front). Without this, streaming
    /// responses arrive with `usage = None`, transcript headers lose the
    /// `- Charged: $…` line, and per-message cost annotations vanish for
    /// streamed sessions (typically the orchestrator).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream_options: Option<OpenAiStreamOptions>,
}

/// OpenAI-spec `stream_options` payload (sent on the wire). Distinct from
/// `crate::openhuman::providers::traits::StreamOptions`, which is the
/// caller-side knob set on `ChatRequest` to toggle agent streaming.
#[derive(Debug, Serialize)]
pub(crate) struct OpenAiStreamOptions {
    pub(crate) include_usage: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct NativeMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<ResponsesInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesInput {
    pub(crate) role: String,
    pub(crate) content: String,
}

// ── Response bodies ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ApiChatResponse {
    pub(crate) choices: Vec<Choice>,
    /// Standard OpenAI usage block.
    #[serde(default)]
    pub(crate) usage: Option<ApiUsage>,
    /// OpenHuman backend metadata (usage + billing summary).
    #[serde(default)]
    pub(crate) openhuman: Option<OpenHumanMeta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Choice {
    pub(crate) message: ResponseMessage,
}

/// Standard OpenAI `usage` block on a chat completion response.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct ApiUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens: u64,
    #[serde(default)]
    pub(crate) total_tokens: u64,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    pub(crate) cached_tokens: u64,
}

/// OpenHuman backend metadata appended to the response JSON.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenHumanMeta {
    #[serde(default)]
    pub(crate) usage: Option<OpenHumanUsage>,
    #[serde(default)]
    pub(crate) billing: Option<OpenHumanBilling>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenHumanUsage {
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    #[allow(dead_code)]
    pub(crate) total_tokens: Option<u64>,
    pub(crate) cached_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenHumanBilling {
    #[serde(default)]
    pub(crate) charged_amount_usd: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ResponseMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// Reasoning/thinking models (e.g. Qwen3, GLM-4) may return their output
    /// in `reasoning_content` instead of `content`. Used as automatic fallback.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub(crate) function_call: Option<Function>,
}

impl ResponseMessage {
    /// Extract text content, falling back to `reasoning_content` when `content`
    /// is missing or empty. Reasoning/thinking models (Qwen3, GLM-4, etc.)
    /// often return their output solely in `reasoning_content`.
    /// Strips `<think>...</think>` blocks that some models (e.g. MiniMax) embed
    /// inline in `content` instead of using a separate field.
    pub(crate) fn effective_content(&self) -> String {
        if let Some(content) = self.content.as_ref().filter(|c| !c.is_empty()) {
            let stripped = super::compatible_parse::strip_think_tags(content);
            if !stripped.is_empty() {
                return stripped;
            }
        }

        self.reasoning_content
            .as_ref()
            .map(|c| super::compatible_parse::strip_think_tags(c))
            .filter(|c| !c.is_empty())
            .unwrap_or_default()
    }

    pub(crate) fn effective_content_optional(&self) -> Option<String> {
        if let Some(content) = self.content.as_ref().filter(|c| !c.is_empty()) {
            let stripped = super::compatible_parse::strip_think_tags(content);
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }

        self.reasoning_content
            .as_ref()
            .map(|c| super::compatible_parse::strip_think_tags(c))
            .filter(|c| !c.is_empty())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) function: Option<Function>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Function {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponse {
    #[serde(default)]
    pub(crate) output: Vec<ResponsesOutput>,
    #[serde(default)]
    pub(crate) output_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesOutput {
    #[serde(default)]
    pub(crate) content: Vec<ResponsesContent>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesContent {
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) text: Option<String>,
}

// ── Streaming types ───────────────────────────────────────────────────────────

/// Server-Sent Event stream chunk for OpenAI-compatible streaming.
#[derive(Debug, Deserialize)]
pub(crate) struct StreamChunkResponse {
    pub(crate) choices: Vec<StreamChoice>,
    #[serde(default)]
    pub(crate) usage: Option<ApiUsage>,
    #[serde(default)]
    pub(crate) openhuman: Option<OpenHumanMeta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamChoice {
    pub(crate) delta: StreamDelta,
    #[allow(dead_code)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamDelta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// Reasoning/thinking models may stream output via `reasoning_content`.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    /// Native tool-call chunks. Each entry is keyed by `index`; the first
    /// chunk for a given index carries `id`/`type`/`function.name`, later
    /// chunks only carry fragments of `function.arguments`.
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamToolCallDelta {
    /// Index of this tool call within the assistant message. Multiple
    /// concurrent tool calls share the same message and are distinguished
    /// by index — not id (which may only appear on the first chunk).
    #[serde(default)]
    pub(crate) index: Option<u32>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default, rename = "type")]
    #[allow(dead_code)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<StreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamToolCallFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Arguments are streamed as a raw JSON string fragment; we accumulate
    /// them as-is and only parse at the end of the stream.
    #[serde(default)]
    pub(crate) arguments: Option<String>,
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
pub(crate) struct StreamingToolCall {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) arguments: String,
    pub(crate) emitted_start: bool,
    pub(crate) emitted_chars: usize,
}
