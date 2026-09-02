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
    /// Bytes of an incomplete UTF-8 sequence carried over from the previous chunk.
    ///
    /// `feed_bytes` is handed arbitrary read boundaries, so a multi-byte character
    /// can straddle two chunks. Decoding each chunk on its own replaced both halves
    /// with U+FFFD, and the JSON still parsed -- the replacement character is legal
    /// inside a string -- so the corruption reached the transcript silently.
    pending: Vec<u8>,
    /// First-seen `schema_version` from a `system` event, if any.
    pub schema_version: Option<String>,
}

/// Decode `bytes` as UTF-8, returning the text and any incomplete trailing
/// sequence to carry into the next chunk.
///
/// Invalid bytes are replaced, as `from_utf8_lossy` would — the stream must not
/// stall on input that will never become valid. The difference is that only the
/// invalid *sequence* is consumed, and decoding then continues, so an incomplete
/// character at the very end is still carried.
///
/// Handing the whole remainder to `from_utf8_lossy` after the first bad byte
/// looks equivalent and is not: it replaces a trailing partial character before
/// its other half has arrived, which is the corruption this parser carries
/// `pending` to avoid. One stray byte earlier in the chunk was enough to bring
/// it back.
fn decode_carrying_tail(bytes: &[u8]) -> (String, Vec<u8>) {
    let mut decoded = String::new();
    let mut rest = bytes;

    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                decoded.push_str(text);
                return (decoded, Vec::new());
            }
            Err(err) => {
                let valid = err.valid_up_to();
                // `valid_up_to` guarantees this prefix is valid UTF-8.
                decoded.push_str(&String::from_utf8_lossy(&rest[..valid]));
                match err.error_len() {
                    // An incomplete tail: hold it for the next chunk.
                    None => return (decoded, rest[valid..].to_vec()),
                    // One replacement per invalid sequence, matching lossy
                    // semantics, then carry on with what follows it.
                    Some(len) => {
                        decoded.push(char::REPLACEMENT_CHARACTER);
                        rest = &rest[valid + len..];
                    }
                }
            }
        }
    }
}

impl StreamJsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a UTF-8 byte chunk and return any events whose terminating
    /// newline arrived in this chunk.
    ///
    /// See [`decode_carrying_tail`] for the decode contract.
    pub fn feed_bytes(&mut self, chunk: &[u8]) -> Vec<ClaudeCodeEvent> {
        // Decode only complete sequences and carry the trailing partial one into the
        // next chunk. `valid_up_to()` is the boundary; anything after it is either an
        // incomplete tail (keep it) or genuinely invalid input (replace it, as before).
        let bytes: &[u8] = if self.pending.is_empty() {
            chunk
        } else {
            self.pending.extend_from_slice(chunk);
            &self.pending[..]
        };
        let (decoded, carry) = decode_carrying_tail(bytes);
        self.pending = carry;
        self.buffer.push_str(&decoded);
        self.flush()
    }

    /// Append a string chunk.
    pub fn feed(&mut self, chunk: &str) -> Vec<ClaudeCodeEvent> {
        self.buffer.push_str(chunk);
        self.flush()
    }

    /// Drain any remaining buffered content. Call on EOF.
    pub fn end(&mut self) -> Vec<ClaudeCodeEvent> {
        // No further chunk is coming, so a held incomplete sequence can never be
        // completed. Release it lossily rather than dropping it: that is what the
        // per-chunk decode produced before the carry-over existed, and silently
        // discarding trailing bytes would be a different kind of data loss than the
        // one this parser is fixing.
        if !self.pending.is_empty() {
            let tail = std::mem::take(&mut self.pending);
            self.buffer.push_str(&String::from_utf8_lossy(&tail));
        }
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
#[path = "stream_parser_tests.rs"]
mod tests;
