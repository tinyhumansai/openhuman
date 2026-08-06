//! Pure, terminal-free transcript reducer for the tabbed terminal UI's Chat tab.
//!
//! [`TranscriptState`] is a plain data structure with **no ratatui / crossterm /
//! IO dependencies** — the renderer ([`super::render`]) reads it and the event
//! loop ([`super::app`]) mutates it, but the state transitions themselves live
//! here so they can be unit-tested without a terminal.
//!
//! The single entry point is [`TranscriptState::apply_event`], which folds a
//! [`WebChannelEvent`] (the same struct the desktop app receives over Socket.IO)
//! into the transcript. Events for a different `client_id` are ignored, so a
//! process-wide broadcast bus can be drained safely.

use crate::core::socketio::WebChannelEvent;

/// The kind of a transcript entry — drives colour / prefix in the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A message the local user sent.
    User,
    /// The assistant's streamed / final reply text.
    Assistant,
    /// The assistant's "thinking" (reasoning) stream — rendered dimmed.
    Thinking,
    /// A tool-call / tool-result status line.
    Tool,
    /// A terminal error (`chat_error`).
    Error,
    /// A local status/system note (never produced by `apply_event`).
    System,
}

/// One line-group in the transcript. `text` accumulates across streaming deltas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub text: String,
}

impl Entry {
    fn new(kind: EntryKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// Accumulated transcript + streaming status for one chat client stream.
#[derive(Debug, Clone)]
pub struct TranscriptState {
    /// Our stream identity. Events whose `client_id` differs are ignored.
    client_id: String,
    /// The rendered transcript, oldest first.
    entries: Vec<Entry>,
    /// True while a turn is in flight (between send and `chat_done`/`chat_error`).
    streaming: bool,
    /// Index into `entries` of the assistant entry currently accumulating text
    /// deltas for the in-flight turn, if any.
    cur_assistant: Option<usize>,
    /// Index into `entries` of the thinking entry currently accumulating
    /// thinking deltas for the in-flight turn, if any.
    cur_thinking: Option<usize>,
}

impl TranscriptState {
    /// Create an empty transcript bound to `client_id`.
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            entries: Vec::new(),
            streaming: false,
            cur_assistant: None,
            cur_thinking: None,
        }
    }

    /// The transcript entries, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether a turn is currently streaming.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Our client stream id.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Record a locally-sent user message and begin a new turn.
    ///
    /// Resets the streaming cursors so the next `text_delta` / `thinking_delta`
    /// opens fresh assistant / thinking entries for this turn.
    pub fn begin_user_turn(&mut self, message: impl Into<String>) {
        let text = message.into();
        log::debug!("[tui] state: begin_user_turn len={}", text.len());
        self.entries.push(Entry::new(EntryKind::User, text));
        self.cur_assistant = None;
        self.cur_thinking = None;
        self.streaming = true;
    }

    /// Push a local system/status note (e.g. "Cancelled", connection info).
    pub fn push_system(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::debug!("[tui] state: push_system len={}", text.len());
        self.entries.push(Entry::new(EntryKind::System, text));
    }

    /// Fold one [`WebChannelEvent`] into the transcript.
    ///
    /// Events whose `client_id` does not match ours are ignored (the web-channel
    /// bus is process-wide). Returns nothing; inspect [`Self::entries`] /
    /// [`Self::is_streaming`] afterwards.
    pub fn apply_event(&mut self, ev: &WebChannelEvent) {
        if ev.client_id != self.client_id {
            log::trace!(
                "[tui] state: ignoring event={} for other client_id={}",
                ev.event,
                ev.client_id
            );
            return;
        }

        match ev.event.as_str() {
            "text_delta" => {
                if let Some(delta) = ev.delta.as_deref() {
                    self.append_assistant(delta);
                }
            }
            "thinking_delta" => {
                if let Some(delta) = ev.delta.as_deref() {
                    self.append_thinking(delta);
                }
            }
            "tool_call" => {
                let name = ev.tool_name.as_deref().unwrap_or("tool");
                let args = ev.args.as_ref().map(summarize_json).unwrap_or_default();
                log::debug!("[tui] state: tool_call {name}");
                self.entries
                    .push(Entry::new(EntryKind::Tool, format!("→ {name}{args}")));
            }
            "tool_result" => {
                let name = ev.tool_name.as_deref().unwrap_or("tool");
                let ok = ev.success.unwrap_or(true);
                let marker = if ok { "✓" } else { "✗" };
                let detail = ev
                    .output
                    .as_deref()
                    .map(truncate_line)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" — {s}"))
                    .unwrap_or_default();
                log::debug!("[tui] state: tool_result {name} ok={ok}");
                self.entries.push(Entry::new(
                    EntryKind::Tool,
                    format!("{marker} {name}{detail}"),
                ));
            }
            "chat_done" => {
                log::debug!(
                    "[tui] state: chat_done full_response={}",
                    ev.full_response.is_some()
                );
                // `full_response` is authoritative — it replaces whatever the
                // streamed text deltas accumulated (they can lag / be partial).
                if let Some(full) = ev.full_response.as_deref() {
                    match self.cur_assistant {
                        Some(idx) => self.entries[idx].text = full.to_string(),
                        None => self
                            .entries
                            .push(Entry::new(EntryKind::Assistant, full.to_string())),
                    }
                }
                self.finish_turn();
            }
            "chat_error" => {
                let msg = ev.message.as_deref().unwrap_or("Unknown error").to_string();
                log::debug!("[tui] state: chat_error {msg}");
                self.entries.push(Entry::new(EntryKind::Error, msg));
                self.finish_turn();
            }
            other => {
                log::trace!("[tui] state: unhandled event={other}");
            }
        }
    }

    fn append_assistant(&mut self, delta: &str) {
        match self.cur_assistant {
            Some(idx) => self.entries[idx].text.push_str(delta),
            None => {
                self.entries
                    .push(Entry::new(EntryKind::Assistant, delta.to_string()));
                self.cur_assistant = Some(self.entries.len() - 1);
            }
        }
    }

    fn append_thinking(&mut self, delta: &str) {
        match self.cur_thinking {
            Some(idx) => self.entries[idx].text.push_str(delta),
            None => {
                self.entries
                    .push(Entry::new(EntryKind::Thinking, delta.to_string()));
                self.cur_thinking = Some(self.entries.len() - 1);
            }
        }
    }

    fn finish_turn(&mut self) {
        self.streaming = false;
        self.cur_assistant = None;
        self.cur_thinking = None;
    }
}

/// One-line, length-capped summary of a JSON value for tool-call args display.
fn summarize_json(value: &serde_json::Value) -> String {
    let rendered = match value {
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => value.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    format!("({})", truncate_line(&rendered))
}

/// Collapse to a single line and cap length so a rogue tool output can't blow
/// up the transcript width.
fn truncate_line(s: &str) -> String {
    const MAX: usize = 120;
    let single = s.replace(['\n', '\r'], " ");
    let trimmed = single.trim();
    if trimmed.chars().count() > MAX {
        let cut: String = trimmed.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &str = "tui-abc123";

    fn ev(event: &str) -> WebChannelEvent {
        WebChannelEvent {
            event: event.to_string(),
            client_id: CLIENT.to_string(),
            thread_id: "thread-1".to_string(),
            ..Default::default()
        }
    }

    fn text_delta(delta: &str) -> WebChannelEvent {
        WebChannelEvent {
            delta: Some(delta.to_string()),
            delta_kind: Some("text".to_string()),
            ..ev("text_delta")
        }
    }

    fn thinking_delta(delta: &str) -> WebChannelEvent {
        WebChannelEvent {
            delta: Some(delta.to_string()),
            delta_kind: Some("thinking".to_string()),
            ..ev("thinking_delta")
        }
    }

    #[test]
    fn text_deltas_accumulate_into_one_assistant_entry() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("hi");
        s.apply_event(&text_delta("Hel"));
        s.apply_event(&text_delta("lo "));
        s.apply_event(&text_delta("world"));

        let assistant: Vec<_> = s
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Assistant)
            .collect();
        assert_eq!(assistant.len(), 1, "deltas must fold into a single entry");
        assert_eq!(assistant[0].text, "Hello world");
        assert!(s.is_streaming(), "still streaming before chat_done");
    }

    #[test]
    fn thinking_and_text_are_separate_entries() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("q");
        s.apply_event(&thinking_delta("let me think"));
        s.apply_event(&text_delta("answer"));

        let kinds: Vec<_> = s.entries().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EntryKind::User, EntryKind::Thinking, EntryKind::Assistant]
        );
        let thinking = s
            .entries()
            .iter()
            .find(|e| e.kind == EntryKind::Thinking)
            .unwrap();
        assert_eq!(thinking.text, "let me think");
    }

    #[test]
    fn thinking_deltas_accumulate_separately_from_text() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("q");
        s.apply_event(&thinking_delta("a"));
        s.apply_event(&thinking_delta("b"));
        s.apply_event(&text_delta("x"));
        s.apply_event(&thinking_delta("c")); // interleaved — same thinking entry
        let thinking: Vec<_> = s
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Thinking)
            .collect();
        assert_eq!(thinking.len(), 1);
        assert_eq!(thinking[0].text, "abc");
    }

    #[test]
    fn chat_done_replaces_streamed_text_with_full_response() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("hi");
        s.apply_event(&text_delta("Hel")); // partial / laggy stream
        let done = WebChannelEvent {
            full_response: Some("Hello, world!".to_string()),
            ..ev("chat_done")
        };
        s.apply_event(&done);

        let assistant: Vec<_> = s
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Assistant)
            .collect();
        assert_eq!(assistant.len(), 1);
        assert_eq!(
            assistant[0].text, "Hello, world!",
            "full_response is authoritative and replaces the streamed text"
        );
        assert!(!s.is_streaming(), "chat_done ends the turn");
    }

    #[test]
    fn chat_done_without_prior_deltas_still_shows_full_response() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("hi");
        let done = WebChannelEvent {
            full_response: Some("Direct answer".to_string()),
            ..ev("chat_done")
        };
        s.apply_event(&done);
        let assistant = s
            .entries()
            .iter()
            .find(|e| e.kind == EntryKind::Assistant)
            .expect("chat_done with full_response must produce an assistant entry");
        assert_eq!(assistant.text, "Direct answer");
        assert!(!s.is_streaming());
    }

    #[test]
    fn chat_error_pushes_error_entry_and_ends_stream() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("hi");
        let err = WebChannelEvent {
            message: Some("rate limited".to_string()),
            error_type: Some("rate_limit".to_string()),
            ..ev("chat_error")
        };
        s.apply_event(&err);
        let error = s
            .entries()
            .iter()
            .find(|e| e.kind == EntryKind::Error)
            .expect("chat_error must produce an error entry");
        assert_eq!(error.text, "rate limited");
        assert!(!s.is_streaming(), "chat_error ends the turn");
    }

    #[test]
    fn events_for_other_client_id_are_ignored() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("hi");
        let before = s.entries().len();
        let foreign = WebChannelEvent {
            client_id: "tui-someone-else".to_string(),
            delta: Some("not mine".to_string()),
            ..WebChannelEvent {
                event: "text_delta".to_string(),
                thread_id: "thread-1".to_string(),
                ..Default::default()
            }
        };
        s.apply_event(&foreign);
        assert_eq!(
            s.entries().len(),
            before,
            "foreign client_id events must not mutate our transcript"
        );
    }

    #[test]
    fn tool_call_and_result_produce_tool_entries() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("do it");
        let call = WebChannelEvent {
            tool_name: Some("web_search".to_string()),
            args: Some(serde_json::json!({"query": "rust ratatui"})),
            ..ev("tool_call")
        };
        s.apply_event(&call);
        let result = WebChannelEvent {
            tool_name: Some("web_search".to_string()),
            success: Some(true),
            output: Some("3 results".to_string()),
            ..ev("tool_result")
        };
        s.apply_event(&result);

        let tools: Vec<_> = s
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Tool)
            .collect();
        assert_eq!(tools.len(), 2);
        assert!(tools[0].text.starts_with("→ web_search"));
        assert!(tools[0].text.contains("rust ratatui"));
        assert!(tools[1].text.starts_with("✓ web_search"));
        assert!(tools[1].text.contains("3 results"));
    }

    #[test]
    fn failed_tool_result_uses_cross_marker() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("do it");
        let result = WebChannelEvent {
            tool_name: Some("run_shell".to_string()),
            success: Some(false),
            output: Some("exit code 1".to_string()),
            ..ev("tool_result")
        };
        s.apply_event(&result);
        let tool = s
            .entries()
            .iter()
            .find(|e| e.kind == EntryKind::Tool)
            .unwrap();
        assert!(tool.text.starts_with("✗ run_shell"));
    }

    #[test]
    fn second_turn_opens_fresh_assistant_entry() {
        let mut s = TranscriptState::new(CLIENT);
        s.begin_user_turn("first");
        s.apply_event(&text_delta("one"));
        s.apply_event(&WebChannelEvent {
            full_response: Some("one".to_string()),
            ..ev("chat_done")
        });
        s.begin_user_turn("second");
        s.apply_event(&text_delta("two"));

        let assistant: Vec<_> = s
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Assistant)
            .collect();
        assert_eq!(assistant.len(), 2, "each turn gets its own assistant entry");
        assert_eq!(assistant[0].text, "one");
        assert_eq!(assistant[1].text, "two");
    }

    #[test]
    fn truncate_line_collapses_newlines_and_caps_length() {
        let long = "a\nb\n".repeat(200);
        let out = truncate_line(&long);
        assert!(!out.contains('\n'));
        assert!(out.chars().count() <= 121, "capped to MAX + ellipsis");
    }
}
