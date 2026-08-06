//! Sequenced session event stream types.
//!
//! Split from the parent types module. Field names use serde renames to match
//! the backend camelCase wire format exactly, and unknown fields are tolerated
//! so the client keeps working against newer server versions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Envelope wrapping every event on the session stream.
///
/// `event` retains the raw JSON payload; [`EventEnvelope::kind`] parses it into
/// a typed [`EventKind`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub at: u64,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "cycleId", default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    /// Raw event payload; shape depends on `event.kind`.
    pub event: Value,
}

impl EventEnvelope {
    /// Parse the raw `event` payload into a typed [`EventKind`].
    pub fn kind(&self) -> EventKind {
        EventKind::from_value(&self.event)
    }
}

/// Typed event payload parsed from [`EventEnvelope::event`].
///
/// `Unknown` preserves the raw value for forward-compatibility with event
/// kinds this client does not yet model.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// A user message was recorded.
    User { body: String },
    /// The assistant produced a final message.
    Assistant { body: String },
    /// A cognitive cycle started.
    CycleStart { cycle_id: Option<String> },
    /// A cognitive cycle ended.
    CycleEnd {
        cycle_id: Option<String>,
        pass_count: Option<u64>,
        duration_ms: Option<u64>,
        error: Option<bool>,
    },
    /// An error occurred during a cycle.
    Error { source: String, message: String },
    /// Streaming assistant token delta (unpersisted, no seq).
    AssistantDelta { delta: String },
    /// Streaming reasoning token delta (unpersisted, no seq).
    ReasoningDelta { delta: String },
    /// Streaming tool-call delta (unpersisted); raw payload preserved.
    ToolCallDelta { value: Value },
    /// An event kind not modelled by this client; raw payload preserved.
    Unknown(Value),
}

impl EventKind {
    /// Parse a raw event object (`{ "kind": ..., ... }`) into a typed kind.
    pub fn from_value(v: &Value) -> EventKind {
        let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
        let str_field = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let opt_str = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let opt_u64 = |k: &str| v.get(k).and_then(Value::as_u64);
        match kind {
            "user" => EventKind::User {
                body: str_field("body"),
            },
            "assistant" => EventKind::Assistant {
                body: str_field("body"),
            },
            "cycle_start" => EventKind::CycleStart {
                cycle_id: opt_str("cycleId"),
            },
            "cycle_end" => EventKind::CycleEnd {
                cycle_id: opt_str("cycleId"),
                pass_count: opt_u64("passCount"),
                duration_ms: opt_u64("durationMs"),
                error: v.get("error").and_then(Value::as_bool),
            },
            "error" => EventKind::Error {
                source: str_field("source"),
                message: str_field("message"),
            },
            "assistant_delta" => EventKind::AssistantDelta {
                delta: str_field("delta"),
            },
            "reasoning_delta" => EventKind::ReasoningDelta {
                delta: str_field("delta"),
            },
            "tool_call_delta" => EventKind::ToolCallDelta { value: v.clone() },
            _ => EventKind::Unknown(v.clone()),
        }
    }
}
