//! Serde request/response structs for the OpenAI-compatible provider.
//!
//! All types in this module are crate-internal (`pub(crate)` or `pub(crate)`
//! as appropriate). External code only sees the public API on
//! [`super::OpenAiCompatibleProvider`].

use serde::{Deserialize, Deserializer, Serialize};

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ApiChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
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
    pub(crate) content: MessageContent,
}

/// OpenAI Chat Completions message `content` — a union of a plain string
/// (text-only, the overwhelming majority of messages) and an array of typed
/// parts when the message carries image attachments.
///
/// Serialises with `#[serde(untagged)]` so the wire shape matches the OpenAI
/// contract exactly: a bare JSON string for text, or a
/// `[{ "type": "text", … }, { "type": "image_url", … }]` array for multimodal
/// messages. Text-only requests stay byte-identical to the legacy wire shape,
/// so this change is transparent for every non-attachment turn.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// One element of a multimodal `content` array.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

/// OpenAI `image_url` payload. `url` accepts either a base64 `data:` URI (what
/// the chat composer produces) or a remote `https://` link.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ImageUrl {
    pub(crate) url: String,
}

/// `[IMAGE:<data-uri>]` marker prefix. Mirrors
/// [`crate::openhuman::agent::multimodal`] — the agent harness embeds image
/// attachments as these markers inside the message text, and the provider
/// layer promotes them back into structured `image_url` parts at the wire
/// boundary. Kept local to avoid a provider→agent dependency cycle.
const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";

impl MessageContent {
    /// Build message content from a raw chat-message string, promoting any
    /// embedded `[IMAGE:<data-uri>]` markers into structured `image_url`
    /// parts. Returns the plain-string [`MessageContent::Text`] arm when no
    /// markers are present, so text-only messages are unchanged on the wire.
    pub(crate) fn from_chat_text(content: &str) -> Self {
        // Fast path: markerless content stays the plain-string arm, byte-identical.
        if !content.contains(IMAGE_MARKER_PREFIX) {
            return MessageContent::Text(content.to_string());
        }

        // Scan left-to-right, emitting `text` and `image_url` parts in the exact
        // order they appear so interleaved prompts (`before [IMAGE:a] after`,
        // `[IMAGE:a] explain`) keep the multimodal sequence the user authored.
        let mut parts: Vec<ContentPart> = Vec::new();
        let mut text_buf = String::new();
        let mut cursor = 0usize;

        while let Some(rel) = content[cursor..].find(IMAGE_MARKER_PREFIX) {
            let start = cursor + rel;
            text_buf.push_str(&content[cursor..start]);

            let marker_start = start + IMAGE_MARKER_PREFIX.len();
            let Some(rel_end) = content[marker_start..].find(']') else {
                // Unterminated marker — keep the remainder as literal text.
                text_buf.push_str(&content[start..]);
                cursor = content.len();
                break;
            };

            let end = marker_start + rel_end;
            let candidate = content[marker_start..end].trim();
            if candidate.is_empty() {
                // `[IMAGE:]` with no payload — keep the literal text, no part.
                text_buf.push_str(&content[start..=end]);
            } else {
                flush_text_part(&mut parts, &mut text_buf);
                parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: candidate.to_string(),
                    },
                });
            }
            cursor = end + 1;
        }
        text_buf.push_str(&content[cursor..]);
        flush_text_part(&mut parts, &mut text_buf);

        // Only empty/invalid markers were present (no image parts) — fall back to
        // the plain-string arm rather than emitting a lone text part.
        if !parts
            .iter()
            .any(|p| matches!(p, ContentPart::ImageUrl { .. }))
        {
            return MessageContent::Text(content.to_string());
        }
        MessageContent::Parts(parts)
    }
}

/// Drain `buf` into a trimmed `ContentPart::Text` when it holds non-whitespace,
/// then clear it. Whitespace-only spans between markers are dropped.
fn flush_text_part(parts: &mut Vec<ContentPart>, buf: &mut String) {
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        parts.push(ContentPart::Text {
            text: trimmed.to_string(),
        });
    }
    buf.clear();
}

impl From<String> for MessageContent {
    fn from(value: String) -> Self {
        MessageContent::Text(value)
    }
}

impl From<&str> for MessageContent {
    fn from(value: &str) -> Self {
        MessageContent::Text(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<NativeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
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
    /// set — see `crate::openhuman::inference::provider::thread_context`).
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
    /// Ollama-specific `options` block (e.g. `{"num_ctx": 32768}`).
    /// Injected by the factory when the provider profile declares a
    /// `num_ctx` override. Ignored (skipped) for non-Ollama providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) options: Option<OllamaOptions>,
    /// OpenAI-compatible `frequency_penalty`. Set to a small positive value on
    /// real requests to damp degenerate repetition loops — without it a model
    /// that starts repeating a line keeps emitting it until the output-token
    /// cap (self-reinforcing decoding). Skipped when `None` so providers that
    /// don't accept it are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) frequency_penalty: Option<f64>,
}

/// Ollama-specific request options passed in the `options` field.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OllamaOptions {
    /// Context window size override. Ollama defaults to 2048 for many
    /// models; setting this ensures the model allocates enough KV-cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) num_ctx: Option<u32>,
}

/// OpenAI-spec `stream_options` payload (sent on the wire). Distinct from
/// `crate::openhuman::inference::provider::traits::StreamOptions`, which is the
/// caller-side knob set on `ChatRequest` to toggle agent streaming.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OpenAiStreamOptions {
    pub(crate) include_usage: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<ToolCall>>,
    /// Chain-of-thought reasoning returned by thinking models (DeepSeek-R1,
    /// Qwen3, GLM-4, etc.) in the previous assistant turn. Per the API
    /// contract it **must** be echoed back verbatim in the next request's
    /// assistant message, or the provider returns HTTP 400.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<ResponsesInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) store: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesInput {
    pub(crate) role: String,
    pub(crate) content: Vec<ResponsesContentPart>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesContentPart {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) text: String,
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

#[derive(Debug, Serialize)]
pub(crate) struct ResponseMessage {
    pub(crate) content: Option<String>,
    /// Reasoning/thinking models may return their chain-of-thought in a
    /// dedicated field instead of (or alongside) `content`. DeepSeek, Qwen3 and
    /// GLM-4 name it `reasoning_content`; OpenRouter and vLLM/SGLang-backed
    /// OpenAI-compatible proxies emit it as `reasoning`. Both names fold into
    /// this single field (see the manual `Deserialize` impl below) — the CoT
    /// must be echoed back verbatim on tool-call turns or thinking models reject
    /// the follow-up request with HTTP 400.
    pub(crate) reasoning_content: Option<String>,
    pub(crate) tool_calls: Option<Vec<ToolCall>>,
    pub(crate) function_call: Option<Function>,
}

// Manual `Deserialize` so that `reasoning` and `reasoning_content` are accepted
// as DISTINCT wire keys and then folded into the single canonical field.
//
// A serde `alias` maps both names onto one field slot, which makes a provider
// that emits BOTH keys in the same object (some OpenRouter / vLLM-SGLang
// proxies do) fail with `duplicate field \`reasoning_content\``, dropping the
// entire response. Deserializing them as separate optional fields tolerates
// any combination; the canonical `reasoning_content` wins when both are present.
impl<'de> Deserialize<'de> for ResponseMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Shadow {
            #[serde(default)]
            content: Option<String>,
            #[serde(default)]
            reasoning_content: Option<String>,
            #[serde(default)]
            reasoning: Option<String>,
            #[serde(default)]
            tool_calls: Option<Vec<ToolCall>>,
            #[serde(default)]
            function_call: Option<Function>,
        }

        let shadow = Shadow::deserialize(deserializer)?;
        Ok(ResponseMessage {
            content: shadow.content,
            reasoning_content: shadow.reasoning_content.or(shadow.reasoning),
            tool_calls: shadow.tool_calls,
            function_call: shadow.function_call,
        })
    }
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) function: Option<Function>,
    /// Provider-specific passthrough metadata attached to a tool call.
    ///
    /// Google's Gemini OpenAI-compat endpoint returns a cryptographically
    /// signed reasoning token here as
    /// `extra_content.google.thought_signature`, and **requires** it echoed
    /// back verbatim on the assistant tool-call turn of every subsequent
    /// request — otherwise it 400s with "Function call is missing a
    /// thought_signature" (TAURI-RUST-4PK). Captured on the response and
    /// re-emitted on the request as an opaque value so any future
    /// `extra_content.*` keys round-trip unchanged. `skip_serializing_if`
    /// keeps the wire body byte-identical for every provider that doesn't
    /// send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) extra_content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug)]
pub(crate) struct StreamDelta {
    pub(crate) content: Option<String>,
    /// Reasoning/thinking models may stream their chain-of-thought via
    /// `reasoning_content` (DeepSeek/Qwen3/GLM-4) or `reasoning`
    /// (OpenRouter, vLLM/SGLang proxies). Both delta field names fold into
    /// this single field (see the manual `Deserialize` impl below).
    pub(crate) reasoning_content: Option<String>,
    /// Native tool-call chunks. Each entry is keyed by `index`; the first
    /// chunk for a given index carries `id`/`type`/`function.name`, later
    /// chunks only carry fragments of `function.arguments`.
    pub(crate) tool_calls: Option<Vec<StreamToolCallDelta>>,
}

// Manual `Deserialize` for the same reason as `ResponseMessage`: a streaming
// delta that carries both `reasoning` and `reasoning_content` must not fail
// with `duplicate field`. They deserialize as distinct keys and fold into the
// canonical `reasoning_content` (canonical wins when both are present).
impl<'de> Deserialize<'de> for StreamDelta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Shadow {
            #[serde(default)]
            content: Option<String>,
            #[serde(default)]
            reasoning_content: Option<String>,
            #[serde(default)]
            reasoning: Option<String>,
            #[serde(default)]
            tool_calls: Option<Vec<StreamToolCallDelta>>,
        }

        let shadow = Shadow::deserialize(deserializer)?;
        Ok(StreamDelta {
            content: shadow.content,
            reasoning_content: shadow.reasoning_content.or(shadow.reasoning),
            tool_calls: shadow.tool_calls,
        })
    }
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
    /// Provider passthrough metadata (Gemini's `extra_content`, carrying
    /// `google.thought_signature`). Arrives on the first chunk for a given
    /// tool-call index; accumulated and re-emitted so the signature survives
    /// the streaming path (TAURI-RUST-4PK).
    #[serde(default)]
    pub(crate) extra_content: Option<serde_json::Value>,
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
    /// First non-null `extra_content` seen for this tool-call index (Gemini's
    /// thought_signature). Re-emitted on the aggregated [`ToolCall`] so it can
    /// be echoed on the next turn (TAURI-RUST-4PK).
    pub(crate) extra_content: Option<serde_json::Value>,
}
