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
        /// The CLI reports a semantic failure either through `subtype` or
        /// through this flag; keying on `subtype` alone misses the second.
        is_error: bool,
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
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let usage = v.get("usage").cloned();
                let total_cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                ClaudeCodeEvent::Result {
                    subtype,
                    is_error,
                    usage,
                    total_cost_usd,
                    raw: v,
                }
            }
            // The CLI emits `{"error":{"message":"…"}}` for an API failure, so
            // reading `error` as a string returns None and the actionable text was
            // replaced by the literal "claude-code error". That placeholder is not
            // empty, so it survived every downstream "is there a diagnosis?" filter
            // and was reported as though it were one — while suppressing the stderr
            // fallback that did hold the cause. An absent message is now empty,
            // which is what makes that fallback reachable.
            "error" => ClaudeCodeEvent::Error {
                message: v
                    .get("error")
                    .and_then(|error| {
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .filter(|message| !message.trim().is_empty())
                            .or_else(|| error.as_str().filter(|message| !message.trim().is_empty()))
                    })
                    .or_else(|| {
                        v.get("message")
                            .and_then(Value::as_str)
                            .filter(|message| !message.trim().is_empty())
                    })
                    .unwrap_or_default()
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
#[path = "stream_parser_tests.rs"]
mod tests;
