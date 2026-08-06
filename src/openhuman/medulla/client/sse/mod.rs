//! Hand-rolled Server-Sent Events parsing and a reconnecting event stream.
//!
//! The backend emits frames of the form:
//!
//! ```text
//! id: 42
//! data: {"seq":42,"at":...,"sessionId":"...","event":{"kind":"assistant","body":"hi"}}
//!
//! : ping
//!
//! ```
//!
//! `id:` sets the replay cursor (persisted events only; deltas omit it),
//! `data:` carries the JSON [`EventEnvelope`], comment lines (`: ping`) are
//! ignored, and a blank line terminates the current frame.

use std::collections::VecDeque;

use futures::stream::{Stream, StreamExt};

use crate::openhuman::medulla::client::error::{ClientError, Result};
use crate::openhuman::medulla::client::types::EventEnvelope;

impl SseParser {
    /// Create an empty parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of decoded text, appending any completed frames to `out`.
    pub fn feed(&mut self, chunk: &str, out: &mut Vec<SseFrame>) {
        self.line_buf.push_str(chunk);
        while let Some(nl) = self.line_buf.find('\n') {
            let mut line = self.line_buf[..nl].to_string();
            // Drain the line plus the newline from the buffer.
            self.line_buf.drain(..=nl);
            if line.ends_with('\r') {
                line.pop();
            }
            self.feed_line(&line, out);
        }
    }

    fn feed_line(&mut self, line: &str, out: &mut Vec<SseFrame>) {
        if line.is_empty() {
            // Blank line terminates the frame.
            if self.got_data || self.id.is_some() {
                out.push(SseFrame {
                    id: self.id.take(),
                    data: std::mem::take(&mut self.data),
                });
            }
            self.got_data = false;
            self.id = None;
            return;
        }
        // Comment line (`: ...`, e.g. `: ping`) — ignore.
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(i) => {
                let v = &line[i + 1..];
                // A single leading space after the colon is stripped.
                (&line[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line, ""),
        };
        match field {
            "id" => {
                if let Ok(seq) = value.trim().parse::<u64>() {
                    self.id = Some(seq);
                }
            }
            "data" => {
                if self.got_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.got_data = true;
            }
            // `event:`, `retry:` and unknown fields are not used here.
            _ => {}
        }
    }
}

impl SeqDedup {
    /// Start from an optional last-seen seq (the reconnect `Last-Event-ID`).
    pub fn new(start: Option<u64>) -> Self {
        Self { cursor: start }
    }

    /// The current cursor, suitable for a `Last-Event-ID` reconnect header.
    pub fn cursor(&self) -> Option<u64> {
        self.cursor
    }

    /// Decide whether a frame with the given seq should be yielded, advancing
    /// the cursor when it does.
    pub fn accept(&mut self, seq: Option<u64>) -> bool {
        match seq {
            None => true,
            Some(s) => {
                if self.cursor.map(|c| s > c).unwrap_or(true) {
                    self.cursor = Some(s);
                    true
                } else {
                    false
                }
            }
        }
    }
}

impl StreamState {
    /// Open (or reopen) the SSE connection using the current cursor.
    async fn connect(&mut self) -> Result<()> {
        if !self.first_connect {
            // Small backoff between reconnect attempts.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        self.first_connect = false;
        let mut req = self
            .http
            .get(&self.url)
            .header(reqwest::header::ACCEPT, "text/event-stream");
        if let Some(cursor) = self.dedup.cursor() {
            req = req.header("Last-Event-ID", cursor.to_string());
        }
        let resp = req.send().await?.error_for_status()?;
        // Map chunks to owned bytes so the stored stream item type stays
        // nameable without depending on `bytes` directly.
        let body = resp.bytes_stream().map(|r| r.map(|b| b.to_vec()));
        self.body = Some(body.boxed());
        Ok(())
    }

    /// Convert a completed frame into a deduped, decoded envelope (if any).
    fn ingest(&mut self, frame: SseFrame) {
        if !self.dedup.accept(frame.id) {
            return;
        }
        let trimmed = frame.data.trim();
        if trimmed.is_empty() {
            return;
        }
        match serde_json::from_str::<EventEnvelope>(trimmed) {
            Ok(env) => self.pending.push_back(Ok(env)),
            Err(e) => self
                .pending
                .push_back(Err(ClientError::Decode(e.to_string()))),
        }
    }

    /// Produce the next stream item, reconnecting as needed. Returns `None`
    /// only when the stream is permanently exhausted (never, in practice —
    /// it reconnects on end-of-body).
    async fn next(&mut self) -> Option<Result<EventEnvelope>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.body.is_none() {
                if let Err(e) = self.connect().await {
                    // Surface the connect error, then retry on the next poll.
                    return Some(Err(e));
                }
            }
            let body = self.body.as_mut().expect("body set above");
            match body.next().await {
                Some(Ok(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut frames = Vec::new();
                    self.parser.feed(&text, &mut frames);
                    for frame in frames {
                        self.ingest(frame);
                    }
                }
                Some(Err(e)) => {
                    self.body = None;
                    return Some(Err(ClientError::Transport(e)));
                }
                None => {
                    // Server closed the connection; reconnect from cursor.
                    self.body = None;
                }
            }
        }
    }
}

/// Build a reconnecting SSE stream of [`EventEnvelope`]s.
///
/// `url` must already include auth (`?token=<jwt>`). The stream reconnects with
/// the `Last-Event-ID` header and de-duplicates replayed frames by seq. Drop
/// the returned stream to stop.
pub fn event_stream(
    http: reqwest::Client,
    url: String,
    last_event_id: Option<u64>,
) -> impl Stream<Item = Result<EventEnvelope>> {
    let state = StreamState {
        http,
        url,
        parser: SseParser::new(),
        dedup: SeqDedup::new(last_event_id),
        pending: VecDeque::new(),
        body: None,
        first_connect: true,
    };
    futures::stream::unfold(state, |mut state| async move {
        state.next().await.map(|item| (item, state))
    })
}

mod types;
pub use types::SeqDedup;
pub use types::SseFrame;
pub use types::SseParser;
use types::StreamState;
