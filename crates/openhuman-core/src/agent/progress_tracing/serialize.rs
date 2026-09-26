//! Content truncation caps, small JSON-value builders, and NDJSON
//! serialization for finished spans.

use crate::config::schema::AgentTracingBackend;

use super::types::TraceSpan;

/// Cap on tool arguments / tool output recorded onto spans when content
/// capture is on. Keeps a single runaway tool result from bloating the trace
/// batch while still giving Langfuse an actionable preview.
pub(super) const MAX_TOOL_CONTENT_CHARS: usize = 4_000;

/// Cap on captured error text (Langfuse observation `statusMessage`).
pub(super) const MAX_ERROR_MESSAGE_CHARS: usize = 500;

/// Cap on captured model request/completion content and subagent
/// prompt/output. Larger than the tool cap because a generation's input is the
/// full message array (system prompt included) — but still bounded so a
/// 100k-token context can't push the ingestion batch past Langfuse's event
/// size limits.
pub(super) const MAX_MODEL_CONTENT_CHARS: usize = 200_000;

/// Keep large model requests as structured messages so Langfuse can render
/// roles and tool calls. Retain the most recent messages within the bound.
pub(super) fn capture_model_content(value: &serde_json::Value) -> serde_json::Value {
    let serialized = value.to_string();
    if serialized.chars().count() <= MAX_MODEL_CONTENT_CHARS {
        return value.clone();
    }
    if let Some(messages) = value.as_array() {
        let mut kept = Vec::new();
        let mut used = 0;
        for message in messages.iter().rev() {
            let mut message = message.clone();
            let size = message.to_string().chars().count();
            if size > MAX_MODEL_CONTENT_CHARS / 2 {
                if let Some(content) = message.get_mut("content") {
                    *content = serde_json::Value::String(format!(
                        "{}…[message content truncated]",
                        truncate_chars(
                            &content
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| content.to_string()),
                            MAX_MODEL_CONTENT_CHARS / 2
                        )
                    ));
                }
            }
            let size = message.to_string().chars().count();
            if used + size > MAX_MODEL_CONTENT_CHARS.saturating_sub(128) {
                break;
            }
            used += size;
            kept.push(message);
        }
        kept.reverse();
        let omitted = messages.len().saturating_sub(kept.len());
        if omitted > 0 {
            kept.insert(
                0,
                serde_json::json!({
                    "role": "system",
                    "content": format!("[{omitted} earlier messages omitted from telemetry]"),
                }),
            );
        }
        return serde_json::Value::Array(kept);
    }
    if let Some(fields) = value.as_object() {
        let mut kept = fields.clone();
        if let Some(content) = kept.get_mut("content") {
            *content = serde_json::Value::String(truncate_chars(
                &content
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| content.to_string()),
                MAX_MODEL_CONTENT_CHARS / 2,
            ));
            if serde_json::Value::Object(kept.clone())
                .to_string()
                .chars()
                .count()
                <= MAX_MODEL_CONTENT_CHARS
            {
                return serde_json::Value::Object(kept);
            }
        }
    }
    serde_json::json!({
        "truncated": true,
        "preview": truncate_chars(&serialized, MAX_MODEL_CONTENT_CHARS / 2),
    })
}

/// Truncate `text` to `max` characters, appending an explicit truncation
/// marker (with the omitted char count) when content was dropped. Returns the
/// input unchanged when it already fits. Slices on char boundaries, so it
/// never panics on multi-byte content.
pub(super) fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((byte_end, _)) => {
            let omitted = text.chars().count() - max;
            format!("{}…[truncated {omitted} chars]", &text[..byte_end])
        }
    }
}

/// Truncate tool-content text to [`MAX_TOOL_CONTENT_CHARS`].
pub(super) fn truncate_capture_text(text: &str) -> String {
    truncate_chars(text, MAX_TOOL_CONTENT_CHARS)
}

pub(super) fn status_of(success: bool) -> super::types::SpanStatus {
    if success {
        super::types::SpanStatus::Ok
    } else {
        super::types::SpanStatus::Error
    }
}

pub(super) fn json_str(s: &str) -> serde_json::Value {
    serde_json::Value::String(s.to_string())
}

pub(super) fn json_u32(n: u32) -> serde_json::Value {
    serde_json::Value::Number(n.into())
}

pub(super) fn json_u64(n: u64) -> serde_json::Value {
    serde_json::Value::Number(n.into())
}

pub(super) fn json_usize(n: usize) -> serde_json::Value {
    serde_json::Value::Number((n as u64).into())
}

pub(super) fn json_f64(n: f64) -> serde_json::Value {
    serde_json::Number::from_f64(n)
        .map(serde_json::Value::Number)
        // NaN/inf can't be JSON numbers — degrade to null rather than panic.
        .unwrap_or(serde_json::Value::Null)
}

/// Serialize spans to NDJSON (one span object per line) in the requested
/// backend envelope. Both backends share the [`TraceSpan`] body; Langfuse
/// wraps each line with a `{"type":"span-create", ...}` observation envelope
/// so it can be POSTed to the Langfuse ingestion API, while OTel emits the
/// bare span. Returns an empty string for an empty slice.
pub(crate) fn spans_to_ndjson(backend: AgentTracingBackend, spans: &[TraceSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        let line = match backend {
            AgentTracingBackend::Otel => serde_json::to_string(span),
            AgentTracingBackend::Langfuse => serde_json::to_string(&serde_json::json!({
                "type": "span-create",
                "body": span,
            })),
        };
        if let Ok(line) = line {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}
