//! The event data model: the [`SessionEvent`] union and its payload structs plus the
//! sequenced [`EventEnvelope`]. These are plain data types; the custom
//! serialization lives in [`super::serde_impl`] and the read-only derivations in
//! [`super::derive`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::super::contract::{VerificationEvidence, WorkerContract};

/// Token accounting reported alongside an inference or task result.
///
/// The cache fields are optional because not every provider reports them and
/// not every wire carries them: absent means "not reported", which the UI
/// renders as no cache segment rather than as zero. Reading a missing number as
/// 0% would claim a cache miss that was never measured.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    /// Tokens consumed by the prompt.
    #[serde(rename = "inputTokens")]
    pub input_tokens: i64,
    /// Tokens produced by the model.
    #[serde(rename = "outputTokens")]
    pub output_tokens: i64,
    /// Prompt tokens served from the provider's cache, when reported.
    #[serde(
        default,
        rename = "cacheReadTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_read_tokens: Option<i64>,
    /// Prompt tokens written into the provider's cache, when reported.
    #[serde(
        default,
        rename = "cacheCreationTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_creation_tokens: Option<i64>,
}

impl Usage {
    /// The share of prompt tokens that came from cache, when both numbers are
    /// known and the prompt was non-empty.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let cached = self.cache_read_tokens? as f64;
        // `inputTokens` is the whole prompt, cached portion included, so the
        // ratio is against it rather than against cached + uncached.
        let total = self.input_tokens as f64;
        (total > 0.0).then(|| (cached / total).clamp(0.0, 1.0))
    }
}

/// A single tool invocation emitted by a completed inference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// The tool's name.
    pub name: String,
    /// The tool arguments as raw JSON; defaults to `null` when absent.
    #[serde(default)]
    pub args: Value,
}

/// The terminal summary of a task: status, digest text, and optional result/usage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskDigest {
    /// The task identifier this digest closes out.
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// The final task status (e.g. `done`).
    pub status: String,
    /// A short human-readable digest of the outcome.
    #[serde(default)]
    pub digest: String,
    /// An optional reference to the full result payload.
    #[serde(default, rename = "resultRef", skip_serializing_if = "Option::is_none")]
    pub result_ref: Option<Value>,
    /// Token usage accrued by the task, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// The task's nesting depth in the cycle tree.
    #[serde(default)]
    pub depth: i64,
    /// Advisory boundaries supplied at delegation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<WorkerContract>,
    /// Verification observations produced in this task's scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<VerificationEvidence>>,
}

/// One node-execution trace entry for the Trace / Overview lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeTrace {
    /// The node that ran.
    pub node: String,
    /// Wall-clock duration in milliseconds.
    #[serde(default)]
    pub ms: i64,
    /// The tool the node invoked, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The operation the node performed, if any (used when no `tool` is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
}

/// The full event union. Unknown kinds ride through in `Unknown`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// An inference call began.
    InferenceStart {
        tier: String,
        op: String,
        model: Option<String>,
    },
    /// An inference call finished, with timing, usage, and any output.
    InferenceEnd {
        tier: String,
        op: String,
        model: Option<String>,
        duration_ms: i64,
        usage: Option<Usage>,
        content: Option<String>,
        reasoning: Option<String>,
        tool_calls: Option<Vec<ToolCall>>,
    },
    /// A streamed tool call started at `index`.
    ToolCallStart { index: i64, name: String },
    /// A chunk of streamed tool-call arguments.
    ToolCallDelta { index: i64, args_delta: String },
    /// A chunk of streamed assistant text.
    AssistantDelta { delta: String },
    /// A chunk of streamed reasoning text.
    ReasoningDelta { delta: String },
    /// A task began.
    TaskStart {
        task_id: String,
        instruction: String,
        depth: i64,
        agent_id: Option<String>,
        contract: Option<WorkerContract>,
    },
    /// A task emitted an event (text, status, etc.).
    TaskEvent {
        task_id: String,
        event_kind: String,
        content: String,
        harness: Option<String>,
    },
    /// A task needs operator attention (e.g. a confirmation).
    TaskAttention {
        task_id: String,
        reason: String,
        content: String,
        question_id: Option<String>,
    },
    /// A task finished; carries its [`TaskDigest`].
    TaskComplete { digest: TaskDigest },
    /// A node-execution trace entry.
    Trace { entry: NodeTrace },
    /// An error surfaced by a source component.
    Error { source: String, message: String },
    /// A cycle began.
    CycleStart { cycle_id: String },
    /// A cycle finished, with its pass count and duration.
    CycleEnd {
        cycle_id: String,
        pass_count: i64,
        duration_ms: i64,
    },
    /// An agent's availability changed.
    AgentStatus {
        agent_id: String,
        availability: String,
        detail: Option<String>,
    },
    /// A session emitted an event on an agent.
    SessionEvent {
        agent_id: String,
        session_id: String,
        event_kind: String,
        content: String,
    },
    /// A peer session's state changed.
    PeerSession {
        agent_id: String,
        session_id: String,
        state: String,
        harness: Option<String>,
    },
    /// A user chat turn.
    User { body: String },
    /// An assistant chat turn.
    Assistant { body: String },
    /// A host effect (opaque JSON).
    Effect { effect: Value },
    /// An unrecognized event kind, preserved verbatim for forward compatibility.
    Unknown {
        kind: String,
        data: Map<String, Value>,
    },
}

/// A sequenced event with its wall-clock timestamp (epoch ms).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    /// Monotonic sequence number within the stream.
    pub seq: u64,
    /// Wall-clock timestamp in epoch milliseconds.
    pub at: i64,
    /// The wrapped event.
    pub event: SessionEvent,
}

impl SessionEvent {
    /// The `kind` discriminator, matching the JSON tag.
    pub fn kind(&self) -> &str {
        match self {
            SessionEvent::InferenceStart { .. } => "inference_start",
            SessionEvent::InferenceEnd { .. } => "inference_end",
            SessionEvent::ToolCallStart { .. } => "tool_call_start",
            SessionEvent::ToolCallDelta { .. } => "tool_call_delta",
            SessionEvent::AssistantDelta { .. } => "assistant_delta",
            SessionEvent::ReasoningDelta { .. } => "reasoning_delta",
            SessionEvent::TaskStart { .. } => "task_start",
            SessionEvent::TaskEvent { .. } => "task_event",
            SessionEvent::TaskAttention { .. } => "task_attention",
            SessionEvent::TaskComplete { .. } => "task_complete",
            SessionEvent::Trace { .. } => "trace",
            SessionEvent::Error { .. } => "error",
            SessionEvent::CycleStart { .. } => "cycle_start",
            SessionEvent::CycleEnd { .. } => "cycle_end",
            SessionEvent::AgentStatus { .. } => "agent_status",
            SessionEvent::SessionEvent { .. } => "session_event",
            SessionEvent::PeerSession { .. } => "peer_session",
            SessionEvent::User { .. } => "user",
            SessionEvent::Assistant { .. } => "assistant",
            SessionEvent::Effect { .. } => "effect",
            SessionEvent::Unknown { kind, .. } => kind,
        }
    }
}
