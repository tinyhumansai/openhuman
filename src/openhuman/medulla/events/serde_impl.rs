//! Custom `Serialize`/`Deserialize` for [`SessionEvent`].
//!
//! `SessionEvent` serializes to a compact `{kind, ...}` JSON object (camelCase keys,
//! null fields dropped) and deserializes any such object — keeping unrecognized
//! kinds as [`SessionEvent::Unknown`] so a newer backend never drops rows on an older
//! TUI. The field-extraction helpers below tolerate missing or ill-typed fields.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::types::SessionEvent;

impl SessionEvent {
    /// Render the event to its compact JSON object, including the `kind` tag and
    /// with null fields dropped to keep the shape TS-friendly.
    fn to_value(&self) -> Value {
        let mut v = match self {
            SessionEvent::InferenceStart { tier, op, model } => {
                json!({ "tier": tier, "op": op, "model": model })
            }
            SessionEvent::InferenceEnd {
                tier,
                op,
                model,
                duration_ms,
                usage,
                content,
                reasoning,
                tool_calls,
            } => json!({
                "tier": tier, "op": op, "model": model, "durationMs": duration_ms,
                "usage": usage, "content": content, "reasoning": reasoning,
                "toolCalls": tool_calls,
            }),
            SessionEvent::ToolCallStart { index, name } => json!({ "index": index, "name": name }),
            SessionEvent::ToolCallDelta { index, args_delta } => {
                json!({ "index": index, "argsDelta": args_delta })
            }
            SessionEvent::AssistantDelta { delta } => json!({ "delta": delta }),
            SessionEvent::ReasoningDelta { delta } => json!({ "delta": delta }),
            SessionEvent::TaskStart {
                task_id,
                instruction,
                depth,
                agent_id,
                contract,
            } => {
                json!({ "taskId": task_id, "instruction": instruction, "depth": depth, "agentId": agent_id, "contract": contract })
            }
            SessionEvent::TaskEvent {
                task_id,
                event_kind,
                content,
                harness,
            } => {
                json!({ "taskId": task_id, "eventKind": event_kind, "content": content, "harness": harness })
            }
            SessionEvent::TaskAttention {
                task_id,
                reason,
                content,
                question_id,
            } => {
                json!({ "taskId": task_id, "reason": reason, "content": content, "questionId": question_id })
            }
            SessionEvent::TaskComplete { digest } => json!({ "digest": digest }),
            SessionEvent::Trace { entry } => json!({ "entry": entry }),
            SessionEvent::Error { source, message } => {
                json!({ "source": source, "message": message })
            }
            SessionEvent::CycleStart { cycle_id } => json!({ "cycleId": cycle_id }),
            SessionEvent::CycleEnd {
                cycle_id,
                pass_count,
                duration_ms,
            } => json!({ "cycleId": cycle_id, "passCount": pass_count, "durationMs": duration_ms }),
            SessionEvent::AgentStatus {
                agent_id,
                availability,
                detail,
            } => json!({ "agentId": agent_id, "availability": availability, "detail": detail }),
            SessionEvent::SessionEvent {
                agent_id,
                session_id,
                event_kind,
                content,
            } => {
                json!({ "agentId": agent_id, "sessionId": session_id, "eventKind": event_kind, "content": content })
            }
            SessionEvent::PeerSession {
                agent_id,
                session_id,
                state,
                harness,
            } => {
                json!({ "agentId": agent_id, "sessionId": session_id, "state": state, "harness": harness })
            }
            SessionEvent::User { body } => json!({ "body": body }),
            SessionEvent::Assistant { body } => json!({ "body": body }),
            SessionEvent::Effect { effect } => json!({ "effect": effect }),
            SessionEvent::Unknown { data, .. } => Value::Object(data.clone()),
        };
        if let Value::Object(map) = &mut v {
            map.insert("kind".into(), Value::String(self.kind().to_string()));
            // Drop nulls to keep the JSON compact and TS-shaped.
            map.retain(|_, val| !val.is_null());
        }
        v
    }
}

impl Serialize for SessionEvent {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

/// Read a required string field, defaulting to `""` when missing or non-string.
fn get_str(m: &Map<String, Value>, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
/// Read an optional string field: missing, non-string, or empty all map to `None`.
fn opt_str(m: &Map<String, Value>, k: &str) -> Option<String> {
    m.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}
/// Read a required integer field, defaulting to `0` when missing or non-integer.
fn get_i64(m: &Map<String, Value>, k: &str) -> i64 {
    m.get(k).and_then(Value::as_i64).unwrap_or(0)
}
/// Deserialize a nested field into `T`, yielding `None` on any decode failure.
fn from_field<T: for<'d> Deserialize<'d>>(m: &Map<String, Value>, k: &str) -> Option<T> {
    m.get(k)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

impl<'de> Deserialize<'de> for SessionEvent {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        let map = value
            .as_object()
            .ok_or_else(|| de::Error::custom("event must be an object"))?;
        let kind = map.get("kind").and_then(Value::as_str).unwrap_or("");
        Ok(match kind {
            "inference_start" => SessionEvent::InferenceStart {
                tier: get_str(map, "tier"),
                op: get_str(map, "op"),
                model: opt_str(map, "model"),
            },
            "inference_end" => SessionEvent::InferenceEnd {
                tier: get_str(map, "tier"),
                op: get_str(map, "op"),
                model: opt_str(map, "model"),
                duration_ms: get_i64(map, "durationMs"),
                usage: from_field(map, "usage"),
                content: opt_str(map, "content"),
                reasoning: opt_str(map, "reasoning"),
                tool_calls: from_field(map, "toolCalls"),
            },
            "tool_call_start" => SessionEvent::ToolCallStart {
                index: get_i64(map, "index"),
                name: get_str(map, "name"),
            },
            "tool_call_delta" => SessionEvent::ToolCallDelta {
                index: get_i64(map, "index"),
                args_delta: get_str(map, "argsDelta"),
            },
            "assistant_delta" => SessionEvent::AssistantDelta {
                delta: get_str(map, "delta"),
            },
            "reasoning_delta" => SessionEvent::ReasoningDelta {
                delta: get_str(map, "delta"),
            },
            "task_start" => SessionEvent::TaskStart {
                task_id: get_str(map, "taskId"),
                instruction: get_str(map, "instruction"),
                depth: get_i64(map, "depth"),
                agent_id: opt_str(map, "agentId"),
                contract: from_field(map, "contract"),
            },
            "task_event" => SessionEvent::TaskEvent {
                task_id: get_str(map, "taskId"),
                event_kind: get_str(map, "eventKind"),
                content: get_str(map, "content"),
                harness: opt_str(map, "harness"),
            },
            "task_attention" => SessionEvent::TaskAttention {
                task_id: get_str(map, "taskId"),
                reason: get_str(map, "reason"),
                content: get_str(map, "content"),
                question_id: opt_str(map, "questionId"),
            },
            "task_complete" => SessionEvent::TaskComplete {
                digest: from_field(map, "digest")
                    .ok_or_else(|| de::Error::custom("task_complete needs digest"))?,
            },
            "trace" => SessionEvent::Trace {
                entry: from_field(map, "entry")
                    .ok_or_else(|| de::Error::custom("trace needs entry"))?,
            },
            "error" => SessionEvent::Error {
                source: get_str(map, "source"),
                message: get_str(map, "message"),
            },
            "cycle_start" => SessionEvent::CycleStart {
                cycle_id: get_str(map, "cycleId"),
            },
            "cycle_end" => SessionEvent::CycleEnd {
                cycle_id: get_str(map, "cycleId"),
                pass_count: get_i64(map, "passCount"),
                duration_ms: get_i64(map, "durationMs"),
            },
            "agent_status" => SessionEvent::AgentStatus {
                agent_id: get_str(map, "agentId"),
                availability: get_str(map, "availability"),
                detail: opt_str(map, "detail"),
            },
            "session_event" => SessionEvent::SessionEvent {
                agent_id: get_str(map, "agentId"),
                session_id: get_str(map, "sessionId"),
                event_kind: get_str(map, "eventKind"),
                content: get_str(map, "content"),
            },
            "peer_session" => SessionEvent::PeerSession {
                agent_id: get_str(map, "agentId"),
                session_id: get_str(map, "sessionId"),
                state: get_str(map, "state"),
                harness: opt_str(map, "harness"),
            },
            "user" => SessionEvent::User {
                body: get_str(map, "body"),
            },
            "assistant" => SessionEvent::Assistant {
                body: get_str(map, "body"),
            },
            "effect" => SessionEvent::Effect {
                effect: map.get("effect").cloned().unwrap_or(Value::Null),
            },
            other => SessionEvent::Unknown {
                kind: other.to_string(),
                data: map.clone(),
            },
        })
    }
}
