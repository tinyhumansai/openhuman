//! Pure, terminal-free transcript reducer for the tabbed terminal UI's Chat tab.
//!
//! [`TranscriptState`] is a plain data structure with **no ratatui / crossterm /
//! IO dependencies** — the renderer ([`super::render`]) reads it and the event
//! loop ([`super::app`]) mutates it, but the state transitions themselves live
//! here so they can be unit-tested without a terminal.
//!
//! The single entry point is [`TranscriptState::apply_event`], which folds a
//! TinyChannels-owned typed [`WebChannelEvent`] (the same wire contract the
//! desktop app receives over Socket.IO) into the transcript. Events for a
//! different `client_id` are ignored, so a
//! process-wide broadcast bus can be drained safely.

use super::activity::{Activity, Status};
use openhuman_rpc::embed::chat_surface::WebChannelEvent;

use crate::event_view::WebEventView;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

static REVISION: AtomicU64 = AtomicU64::new(1);
fn revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed)
}

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
    pub(crate) revision: u64,
    pub(crate) activity: Option<Activity>,
    pub(crate) expanded: bool,
}

impl Entry {
    fn new(kind: EntryKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            revision: revision(),
            activity: None,
            expanded: false,
        }
    }
}

/// Accumulated transcript + streaming status for one chat client stream.
#[derive(Debug, Clone)]
pub struct TranscriptState {
    /// Our stream identity. Events whose `client_id` differs are ignored.
    client_id: String,
    /// Active conversation. Events from another thread must not leak into it.
    thread_id: String,
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
    activity_index: HashMap<String, usize>,
    sequences: HashMap<String, u64>,
    viewport_revision: u64,
    viewport_changes: VecDeque<(u64, usize)>,
}

impl TranscriptState {
    /// Create an empty transcript bound to `client_id`.
    pub fn new(client_id: impl Into<String>) -> Self {
        let viewport_revision = revision();
        Self {
            client_id: client_id.into(),
            thread_id: String::new(),
            entries: Vec::new(),
            streaming: false,
            cur_assistant: None,
            cur_thinking: None,
            activity_index: HashMap::new(),
            sequences: HashMap::new(),
            viewport_revision,
            viewport_changes: VecDeque::from([(viewport_revision, 0)]),
        }
    }

    /// The transcript entries, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(crate) fn viewport_revision(&self) -> u64 {
        self.viewport_revision
    }

    /// Changes after a known snapshot; an expired or foreign snapshot requires rebuilding.
    pub(crate) fn viewport_changes_since(&self, prior: u64) -> Option<Vec<usize>> {
        if prior == self.viewport_revision {
            return Some(Vec::new());
        }
        let position = self
            .viewport_changes
            .iter()
            .position(|(token, _)| *token == prior)?;
        let mut indices: Vec<_> = self
            .viewport_changes
            .iter()
            .skip(position + 1)
            .map(|(_, index)| *index)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        Some(indices)
    }

    fn viewport_changed(&mut self, index: usize) {
        self.viewport_revision = revision();
        self.viewport_changes
            .push_back((self.viewport_revision, index));
        if self.viewport_changes.len() > 128 {
            self.viewport_changes.pop_front();
        }
    }

    fn viewport_reset(&mut self) {
        self.viewport_changes.clear();
        self.viewport_changed(0);
    }

    /// Whether a turn is currently streaming.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Our client stream id.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn set_thread(&mut self, thread_id: impl Into<String>) {
        self.thread_id = thread_id.into();
    }

    pub fn clear(&mut self) {
        self.viewport_reset();
        self.entries.clear();
        self.activity_index.clear();
        self.cur_assistant = None;
        self.cur_thinking = None;
    }

    pub fn last_assistant(&self) -> Option<&str> {
        if self.streaming {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.kind == EntryKind::Assistant)
            .map(|entry| entry.text.as_str())
    }

    pub fn export_markdown(&self) -> String {
        let mut out = String::from("# OpenHuman transcript\n\n");
        for entry in &self.entries {
            let title = match entry.kind {
                EntryKind::User => "You",
                EntryKind::Assistant => "OpenHuman",
                EntryKind::Thinking => "Reasoning",
                EntryKind::Tool => "Tool",
                EntryKind::Error => "Error",
                EntryKind::System => "System",
            };
            out.push_str("## ");
            out.push_str(title);
            out.push_str("\n\n");
            out.push_str(&entry.text);
            out.push_str("\n\n");
        }
        out
    }

    /// Replace the view with a newest-first `threads.transcript_get` page.
    pub fn load_transcript(&mut self, value: &serde_json::Value) {
        self.viewport_reset();
        self.entries.clear();
        self.activity_index.clear();
        self.sequences.clear();
        let page = super::cockpit::unwrap_rpc(value);
        let Some(items) = page.get("items").and_then(serde_json::Value::as_array) else {
            return;
        };
        for item in items.iter().rev() {
            self.push_projected_item(item);
        }
        self.finish_turn();
    }

    /// Record a locally-sent user message and begin a new turn.
    ///
    /// Resets the streaming cursors so the next `text_delta` / `thinking_delta`
    /// opens fresh assistant / thinking entries for this turn.
    pub fn begin_user_turn(&mut self, message: impl Into<String>) {
        self.viewport_changed(self.entries.len());
        let text = message.into();
        log::debug!("[tui] state: begin_user_turn len={}", text.len());
        self.entries.push(Entry::new(EntryKind::User, text));
        self.cur_assistant = None;
        self.cur_thinking = None;
        self.streaming = true;
    }

    /// Push a local system/status note (e.g. "Cancelled", connection info).
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.viewport_changed(self.entries.len());
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
        let ev = WebEventView::from_event(ev);
        self.apply_event_view(&ev);
    }

    pub(crate) fn apply_event_view(&mut self, ev: &WebEventView) {
        if ev.client_id != self.client_id
            || (!self.thread_id.is_empty() && ev.thread_id != self.thread_id)
        {
            log::trace!(
                "[tui] state: ignoring event={} for other client_id={}",
                ev.event,
                ev.client_id
            );
            return;
        }

        if let Some(seq) = ev.seq {
            if !ev.request_id.is_empty() {
                if self
                    .sequences
                    .get(&ev.request_id)
                    .is_some_and(|prior| seq <= *prior)
                {
                    return;
                }
                self.sequences.insert(ev.request_id.clone(), seq);
            }
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
            "tool_call" | "tool_result" | "tool_args_delta" => self.apply_activity(ev, false),
            event if event.starts_with("subagent_") => self.apply_activity(ev, true),
            "artifact_pending" | "artifact_ready" | "artifact_failed" => {
                self.viewport_changed(self.entries.len());
                let status = ev.event.trim_start_matches("artifact_");
                let detail = ev
                    .message
                    .as_deref()
                    .or(ev.output.as_deref())
                    .map(truncate_line)
                    .unwrap_or_default();
                let suffix = if detail.is_empty() {
                    String::new()
                } else {
                    format!(" — {detail}")
                };
                self.entries.push(Entry::new(
                    EntryKind::Tool,
                    format!("artifact · {status}{suffix}"),
                ));
            }
            "chat_done" | "channel_outbound" => {
                log::debug!(
                    "[tui] state: {} full_response={}",
                    ev.event,
                    ev.full_response.is_some()
                );
                // `full_response` is authoritative — it replaces whatever the
                // streamed text deltas accumulated (they can lag / be partial).
                if let Some(full) = ev.full_response.as_deref() {
                    self.viewport_changed(self.cur_assistant.unwrap_or(self.entries.len()));
                    match self.cur_assistant {
                        Some(idx) => {
                            self.entries[idx].text = full.to_string();
                            self.entries[idx].revision = revision();
                        }
                        None => self
                            .entries
                            .push(Entry::new(EntryKind::Assistant, full.to_string())),
                    }
                }
                self.finish_turn();
            }
            "chat_error" => {
                self.viewport_changed(self.entries.len());
                let mut msg = ev.message.as_deref().unwrap_or("Unknown error").to_string();
                if ev.error_retryable == Some(true) {
                    msg.push_str(" · retryable");
                }
                if let Some(delay) = ev.error_retry_after_ms {
                    msg.push_str(&format!(" after {}s", delay.div_ceil(1000)));
                }
                log::debug!("[tui] state: chat_error {msg}");
                self.entries.push(Entry::new(EntryKind::Error, msg));
                let mut changed = Vec::new();
                for (index, entry) in self.entries.iter_mut().enumerate() {
                    if let Some(activity) = &mut entry.activity {
                        if activity.status == Status::Running && !activity.child {
                            activity.status = if ev.error_type.as_deref() == Some("cancelled") {
                                Status::Cancelled
                            } else {
                                Status::Unknown
                            };
                            entry.text = activity.summary();
                            entry.revision = revision();
                            changed.push(index);
                        }
                    }
                }
                for index in changed {
                    self.viewport_changed(index);
                }
                self.finish_turn();
            }
            other => match super::state_events::disposition(other) {
                super::state_events::Disposition::Unknown => {
                    log::debug!("[tui] state: unknown event={other} (not in the web contract)");
                }
                _ => log::trace!("[tui] state: ignored event={other}"),
            },
        }
    }

    fn append_assistant(&mut self, delta: &str) {
        self.viewport_changed(self.cur_assistant.unwrap_or(self.entries.len()));
        match self.cur_assistant {
            Some(idx) => {
                self.entries[idx].text.push_str(delta);
                self.entries[idx].revision = revision();
            }
            None => {
                self.entries
                    .push(Entry::new(EntryKind::Assistant, delta.to_string()));
                self.cur_assistant = Some(self.entries.len() - 1);
            }
        }
    }

    fn append_thinking(&mut self, delta: &str) {
        self.viewport_changed(self.cur_thinking.unwrap_or(self.entries.len()));
        match self.cur_thinking {
            Some(idx) => {
                self.entries[idx].text.push_str(delta);
                self.entries[idx].revision = revision();
            }
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

    pub(crate) fn toggle_entry(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.expanded = !entry.expanded;
            entry.revision = revision();
            self.viewport_changed(index);
        }
    }

    fn apply_activity(&mut self, ev: &WebEventView, child: bool) {
        let identity = if child {
            ev.subagent
                .as_ref()
                .and_then(|detail| detail.task_id.as_deref())
                .or(ev.skill_id.as_deref())
        } else {
            ev.tool_call_id.as_deref()
        };
        let key = identity.map(|id| {
            format!(
                "{}:{}:{id}",
                ev.request_id,
                if child { "child" } else { "tool" }
            )
        });
        let existing = key
            .as_ref()
            .and_then(|key| self.activity_index.get(key).copied())
            .or_else(|| {
                if identity.is_none() && ev.event == "tool_result" {
                    self.entries.iter().rposition(|entry| {
                        entry.activity.as_ref().is_some_and(|a| {
                            !a.child
                                && a.status == Status::Running
                                && Some(a.label.as_str()) == ev.tool_name.as_deref()
                        })
                    })
                } else {
                    None
                }
            });
        let index = existing.unwrap_or_else(|| {
            let id = key
                .clone()
                .unwrap_or_else(|| format!("{}:legacy:{}", ev.request_id, self.entries.len()));
            let mut entry = Entry::new(EntryKind::Tool, "");
            entry.activity = Some(Activity::from_event(id.clone(), ev, child));
            let index = self.entries.len();
            self.entries.push(entry);
            self.activity_index.insert(id, index);
            index
        });
        let entry = &mut self.entries[index];
        let activity = entry.activity.as_mut().unwrap();
        if ev.event == "tool_args_delta" {
            if let Some(delta) = &ev.delta {
                activity.args =
                    super::activity::bounded(&format!("{}{delta}", activity.args), 4096);
            }
        }
        activity.update(ev);
        entry.text = activity.summary();
        entry.revision = revision();
        self.viewport_changed(index);
    }

    fn push_projected_item(&mut self, item: &serde_json::Value) {
        match item.get("kind").and_then(serde_json::Value::as_str) {
            Some("userMessage") => {
                let content = item
                    .get("displayContent")
                    .or_else(|| item.get("content"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let text = label_voice_note(content, item);
                self.entries.push(Entry::new(EntryKind::User, text));
            }
            Some("assistantMessage") => {
                let content = item
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let text = label_voice_note(content, item);
                self.entries.push(Entry::new(EntryKind::Assistant, text));
            }
            Some("reasoning") => self.entries.push(Entry::new(
                EntryKind::Thinking,
                item.get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            )),
            Some("toolCall") => {
                let name = item
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Tool");
                let id = item
                    .get("callId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("legacy");
                let mut entry = Entry::new(EntryKind::Tool, "");
                let activity = Activity {
                    id: format!("history:{id}"),
                    label: name.into(),
                    status: match item.get("status").and_then(serde_json::Value::as_str) {
                        Some("success") => Status::Success,
                        Some("error") => Status::Error,
                        _ => Status::Unknown,
                    },
                    args: item
                        .get("args")
                        .map(|args| super::activity::bounded(&args.to_string(), 4096))
                        .unwrap_or_default(),
                    output: super::activity::bounded(
                        item.get("result")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("No stored output"),
                        65536,
                    ),
                    history: String::new(),
                    elapsed_ms: None,
                    child: false,
                    tools: 0,
                };
                entry.text = activity.summary();
                entry.activity = Some(activity);
                self.entries.push(entry);
            }
            Some("interruptedPartial") => self.entries.push(Entry::new(
                EntryKind::Assistant,
                item.get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            )),
            Some("subagent") => {
                let id = item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Subagent");
                let label = item
                    .get("agentId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(id);
                let items = item.get("items").and_then(serde_json::Value::as_array);
                let output = items
                    .map(|items| {
                        items
                            .iter()
                            .map(|nested| {
                                nested
                                    .get("content")
                                    .or_else(|| nested.get("text"))
                                    .or_else(|| nested.get("result"))
                                    .or_else(|| nested.get("name"))
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let activity = Activity {
                    id: format!("history:child:{id}"),
                    label: label.into(),
                    status: match item.get("status").and_then(serde_json::Value::as_str) {
                        Some("completed") => Status::Success,
                        Some("failed" | "incomplete") => Status::Error,
                        Some("interrupted") => Status::Cancelled,
                        _ => Status::Unknown,
                    },
                    args: String::new(),
                    output: super::activity::bounded(&output, 65536),
                    history: String::new(),
                    elapsed_ms: None,
                    child: true,
                    tools: items
                        .map(|items| {
                            items
                                .iter()
                                .filter(|nested| {
                                    nested.get("kind").and_then(serde_json::Value::as_str)
                                        == Some("toolCall")
                                })
                                .count() as u64
                        })
                        .unwrap_or(0),
                };
                let mut entry = Entry::new(EntryKind::Tool, activity.summary());
                entry.activity = Some(activity);
                self.entries.push(entry);
            }
            Some("compaction") => self.entries.push(Entry::new(
                EntryKind::System,
                "Conversation context compacted",
            )),
            _ => {}
        }
    }
}

fn label_voice_note(content: &str, item: &serde_json::Value) -> String {
    let Some(voice_note) = item.pointer("/extraMetadata/voiceNote") else {
        return content.to_string();
    };
    let has_note_id = voice_note
        .get("noteId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|note_id| !note_id.is_empty());
    if !has_note_id {
        return content.to_string();
    }

    let label = voice_note
        .get("durationMs")
        .and_then(serde_json::Value::as_u64)
        .map(format_short_duration)
        .map(|duration| format!("[voice note {duration}]"))
        .unwrap_or_else(|| "[voice note]".to_string());
    format!("{label} {content}")
}

fn format_short_duration(ms: u64) -> String {
    let seconds = ms.saturating_add(500) / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
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
#[path = "state_tests.rs"]
mod tests;
