use tinyinference_llm::usage::Usage;
use tinytools_agent::dialect::NativeToolCall;

/// Token usage returned by a provider: the vendor [`Usage`] token counts plus
/// the host-owned billing the vendor type deliberately does not carry.
///
/// `usage` is the single token-count representation threaded everywhere
/// (cache reads/writes, reasoning tokens and the context window all have
/// homes on it). Only the provider-charged amount stays host data: it is kept
/// as an exact `f64` USD value (the vendor `ChargedAmount` is integer micro
/// units and would round the persisted cost rows). `Deref` exposes the
/// vendor fields (`input_tokens`, `output_tokens`, `cache_creation_tokens`,
/// `reasoning_tokens`, ...) directly.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BilledUsage {
    /// Vendor token counts for the call.
    pub usage: Usage,
    /// Amount billed for this request in USD (from
    /// `openhuman.billing.charged_amount_usd`). Zero when unavailable.
    pub charged_amount_usd: f64,
}

impl BilledUsage {
    /// Plain token counts with no cache breakdown, context window or charge.
    pub fn from_counts(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            usage: Usage::new(input_tokens, output_tokens),
            charged_amount_usd: 0.0,
        }
    }

    /// Input tokens served from the provider prompt/KV cache.
    pub fn cached_input_tokens(&self) -> u64 {
        self.usage.cache_read_tokens
    }

    /// Model context window in tokens; `0` when unknown.
    pub fn context_window(&self) -> u64 {
        self.usage.context_window_tokens.unwrap_or(0)
    }

    /// Sets the cache-read token count.
    pub fn with_cached_input_tokens(mut self, tokens: u64) -> Self {
        self.usage.cache_read_tokens = tokens;
        self
    }

    /// Sets the context window (`0` means unknown).
    pub fn with_context_window(mut self, tokens: u64) -> Self {
        self.usage.context_window_tokens = (tokens > 0).then_some(tokens);
        self
    }

    /// Sets the cache-creation (write) token count.
    pub fn with_cache_creation_tokens(mut self, tokens: u64) -> Self {
        self.usage.cache_creation_tokens = tokens;
        self
    }

    /// Sets the reasoning/thinking token count.
    pub fn with_reasoning_tokens(mut self, tokens: u64) -> Self {
        self.usage.reasoning_tokens = tokens;
        self
    }

    /// Sets the provider-charged USD amount.
    pub fn with_charged_usd(mut self, usd: f64) -> Self {
        self.charged_amount_usd = usd;
        self
    }
}

impl std::ops::Deref for BilledUsage {
    type Target = Usage;
    fn deref(&self) -> &Usage {
        &self.usage
    }
}

/// An LLM response that may contain text, tool calls, or both.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// Text content of the response (may be empty if only tool calls).
    pub text: Option<String>,
    /// Tool calls requested by the LLM.
    pub tool_calls: Vec<NativeToolCall>,
    /// Token usage info from the provider (if available).
    pub usage: Option<BilledUsage>,
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

impl ChatResponse {}

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
/// is meant for (TAURI-RUST-C62).
///
/// `16384` sits comfortably above any realistic single agent turn — `max_tokens`
/// is an upper bound, not a forced length, so the model still stops at its
/// natural end well below the cap on normal turns — while cutting the
/// reservation 4× versus a 64k window.
pub const AGENT_TURN_MAX_OUTPUT_TOKENS: u32 = 16384;
