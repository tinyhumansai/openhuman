//! Line-buffered JSONL parser for `claude --output-format stream-json`.
//!
//! The CLI writes one JSON object per line on stdout. Each object has a
//! `type` discriminator (`system`, `user`, `assistant`, `stream_event`,
//! `result`, `error`, `rate_limit_event`). We keep variants permissive
//! (everything is `serde_json::Value`) so a minor CLI schema bump does
//! not break the parser — the event mapper interprets what it knows.

use serde_json::Value;

/// One decoded event from the `claude` CLI stdout stream.
#[derive(Debug, Clone)]
pub enum ClaudeCodeEvent {
    System {
        session_id: Option<String>,
        schema_version: Option<String>,
        raw: Value,
    },
    User {
        message: Value,
    },
    Assistant {
        message: Value,
    },
    StreamEvent {
        event: Value,
    },
    RateLimit {
        raw: Value,
    },
    Result {
        subtype: Option<String>,
        usage: Option<Value>,
        total_cost_usd: Option<f64>,
        raw: Value,
    },
    Error {
        message: String,
    },
    /// JSONL line that failed to parse. Kept so the driver can log without
    /// dropping silently. Not surfaced as a `ProviderDelta`.
    ParseError {
        line: String,
        reason: String,
    },
}

/// Stateful parser that takes byte chunks from `proc.stdout` and emits
/// fully-formed events on each newline.
#[derive(Debug, Default)]
pub struct StreamJsonParser {
    buffer: String,
    /// First-seen `schema_version` from a `system` event, if any.
    pub schema_version: Option<String>,
}

impl StreamJsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a UTF-8 byte chunk and return any events whose terminating
    /// newline arrived in this chunk.
    pub fn feed_bytes(&mut self, chunk: &[u8]) -> Vec<ClaudeCodeEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        self.flush()
    }

    /// Append a string chunk.
    pub fn feed(&mut self, chunk: &str) -> Vec<ClaudeCodeEvent> {
        self.buffer.push_str(chunk);
        self.flush()
    }

    /// Drain any remaining buffered content. Call on EOF.
    pub fn end(&mut self) -> Vec<ClaudeCodeEvent> {
        if !self.buffer.is_empty() && !self.buffer.ends_with('\n') {
            self.buffer.push('\n');
        }
        self.flush()
    }

    fn flush(&mut self) -> Vec<ClaudeCodeEvent> {
        let mut out = Vec::new();
        loop {
            let Some(nl) = self.buffer.find('\n') else {
                break;
            };
            let line = self.buffer[..nl].trim().to_string();
            self.buffer.drain(..=nl);
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => out.push(self.decode(v)),
                Err(e) => out.push(ClaudeCodeEvent::ParseError {
                    line,
                    reason: e.to_string(),
                }),
            }
        }
        out
    }

    fn decode(&mut self, v: Value) -> ClaudeCodeEvent {
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "system" => {
                let session_id = v
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let schema_version = v
                    .get("schema_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(sv) = &schema_version {
                    if self.schema_version.is_none() {
                        self.schema_version = Some(sv.clone());
                    }
                }
                ClaudeCodeEvent::System {
                    session_id,
                    schema_version,
                    raw: v,
                }
            }
            "user" => ClaudeCodeEvent::User {
                message: v.get("message").cloned().unwrap_or(Value::Null),
            },
            "assistant" => ClaudeCodeEvent::Assistant {
                message: v.get("message").cloned().unwrap_or(Value::Null),
            },
            "stream_event" => ClaudeCodeEvent::StreamEvent {
                event: v.get("event").cloned().unwrap_or(Value::Null),
            },
            "rate_limit_event" => ClaudeCodeEvent::RateLimit { raw: v },
            "result" => {
                let subtype = v.get("subtype").and_then(Value::as_str).map(str::to_string);
                let usage = v.get("usage").cloned();
                let total_cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                ClaudeCodeEvent::Result {
                    subtype,
                    usage,
                    total_cost_usd,
                    raw: v,
                }
            }
            "error" => ClaudeCodeEvent::Error {
                message: v
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("claude-code error")
                    .to_string(),
            },
            other => ClaudeCodeEvent::ParseError {
                line: v.to_string(),
                reason: format!("unknown event type `{other}`"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_chunk() {
        let mut p = StreamJsonParser::new();
        let chunk = r#"{"type":"system","session_id":"s1","schema_version":"2.0"}
{"type":"assistant","message":{"type":"content_block_start","index":0,"content_block":{"type":"text"}}}
"#;
        let events = p.feed(chunk);
        assert_eq!(events.len(), 2);
        assert_eq!(p.schema_version.as_deref(), Some("2.0"));
        assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
        assert!(matches!(events[1], ClaudeCodeEvent::Assistant { .. }));
    }

    #[test]
    fn handles_split_lines_across_chunks() {
        let mut p = StreamJsonParser::new();
        assert!(p.feed("{\"type\":\"system\"").is_empty());
        assert!(p.feed(",\"session_id\":\"s1\"}").is_empty());
        let events = p.feed("\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
    }

    #[test]
    fn flushes_trailing_line_on_end() {
        let mut p = StreamJsonParser::new();
        assert!(p
            .feed(r#"{"type":"result","subtype":"success"}"#)
            .is_empty());
        let events = p.end();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ClaudeCodeEvent::Result { .. }));
    }

    #[test]
    fn unknown_type_becomes_parse_error() {
        let mut p = StreamJsonParser::new();
        let events = p.feed("{\"type\":\"weird\"}\n");
        assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
    }

    #[test]
    fn bad_json_becomes_parse_error() {
        let mut p = StreamJsonParser::new();
        let events = p.feed("not json\n");
        assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
    }
}
