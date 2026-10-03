//! Per-thread batching of committed turns.
//!
//! A [`ConversationBuffer`] holds the turns of each thread that have not been
//! stored yet. [`ConversationBuffer::push`] hands back a [`Batch`] once a
//! thread holds `batch_turns` turns; [`ConversationBuffer::take_idle`] hands
//! back every thread that has been quiet for `idle_secs`. Pure data: no I/O,
//! no clock (callers pass `now`).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use tinymemory::{
    MemoryMeta, Role, SourceKind, SourceRef, StoreItem, ToolCallRef, Turn, TurnRange,
};

/// One committed exchange: the user's message and the assistant's reply.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedTurn {
    /// The thread it belongs to.
    pub thread_id: String,
    /// The agent that answered.
    pub agent_id: Option<String>,
    /// The agent's working folder.
    pub workspace: Option<String>,
    /// The channel the turn arrived on.
    pub channel: Option<String>,
    /// What the user said.
    pub user: String,
    /// What the assistant replied.
    pub assistant: String,
    /// Tool calls made while answering (name and id only).
    pub tool_calls: Vec<ToolCallRef>,
    /// When the turn committed.
    pub at: DateTime<Utc>,
}

/// The tag a conversation item from `channel` carries.
#[must_use]
pub fn channel_tag(channel: &str) -> String {
    format!("channel:{}", channel.to_ascii_lowercase())
}

/// Turns of one thread ready to be stored as one `Conversation` item.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// The thread.
    pub thread_id: String,
    /// Index of the first turn in the batch.
    pub first: u32,
    /// The turns, oldest first.
    pub turns: Vec<CommittedTurn>,
}

impl Batch {
    /// Index of the last turn in the batch.
    #[must_use]
    pub fn last(&self) -> u32 {
        self.first + u32::try_from(self.turns.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// The `Conversation` item this batch stores.
    ///
    /// Meta carries `thread_id`, `agent_id` and `workspace` (from the latest
    /// turn that has them), `turns`, a `channel:<name>` tag, and
    /// `source = conversation:<thread_id>`.
    /// Tool calls ride on each assistant turn by name and id; arguments never
    /// enter the item.
    #[must_use]
    pub fn into_item(self) -> StoreItem {
        let last = self.last();
        let agent_id = self
            .turns
            .iter()
            .rev()
            .find_map(|turn| turn.agent_id.clone());
        let workspace = self
            .turns
            .iter()
            .rev()
            .find_map(|turn| turn.workspace.clone());
        let observed_at = self.turns.last().map(|turn| turn.at);
        let tags: Vec<String> = self
            .turns
            .iter()
            .rev()
            .find_map(|turn| turn.channel.as_deref())
            .map(|channel| vec![channel_tag(channel)])
            .unwrap_or_default();
        let mut turns = Vec::with_capacity(self.turns.len() * 2);
        for turn in self.turns {
            if !turn.user.trim().is_empty() {
                turns.push(Turn {
                    role: Role::User,
                    text: turn.user,
                    at: Some(turn.at),
                    tool_calls: Vec::new(),
                });
            }
            if !turn.assistant.trim().is_empty() || !turn.tool_calls.is_empty() {
                let text = if turn.assistant.trim().is_empty() {
                    "(tool calls only)".to_string()
                } else {
                    turn.assistant
                };
                turns.push(Turn {
                    role: Role::Assistant,
                    text,
                    at: Some(turn.at),
                    tool_calls: turn.tool_calls,
                });
            }
        }
        StoreItem::Conversation {
            turns,
            meta: MemoryMeta {
                workspace,
                thread_id: Some(self.thread_id.clone()),
                turns: Some(TurnRange {
                    first: self.first,
                    last,
                }),
                agent_id,
                source: SourceRef {
                    kind: SourceKind::Conversation,
                    id: Some(self.thread_id),
                },
                observed_at,
                tags,
                ..MemoryMeta::default()
            },
        }
    }
}

#[derive(Debug, Default)]
struct PendingThread {
    first: u32,
    turns: Vec<CommittedTurn>,
    last_activity: Option<DateTime<Utc>>,
}

/// Unstored turns, per thread.
#[derive(Debug, Default)]
pub struct ConversationBuffer {
    threads: HashMap<String, PendingThread>,
}

impl ConversationBuffer {
    /// Adds `turn` as turn number `index` of its thread. Returns the thread's
    /// batch when it now holds `batch_turns` turns.
    pub fn push(&mut self, turn: CommittedTurn, index: u32, batch_turns: u32) -> Option<Batch> {
        let thread_id = turn.thread_id.clone();
        let pending = self.threads.entry(thread_id.clone()).or_default();
        if pending.turns.is_empty() {
            pending.first = index;
        }
        pending.last_activity = Some(turn.at);
        pending.turns.push(turn);
        if pending.turns.len() >= batch_turns.max(1) as usize {
            return self.take(&thread_id);
        }
        None
    }

    /// Removes and returns every thread idle for at least `idle_secs` at `now`.
    pub fn take_idle(&mut self, now: DateTime<Utc>, idle_secs: u64) -> Vec<Batch> {
        let idle = Duration::seconds(i64::try_from(idle_secs).unwrap_or(i64::MAX));
        let ready: Vec<String> = self
            .threads
            .iter()
            .filter(|(_, pending)| {
                pending
                    .last_activity
                    .is_some_and(|last| now.signed_duration_since(last) >= idle)
            })
            .map(|(thread_id, _)| thread_id.clone())
            .collect();
        ready
            .into_iter()
            .filter_map(|thread_id| self.take(&thread_id))
            .collect()
    }

    /// Removes and returns every pending thread.
    pub fn take_all(&mut self) -> Vec<Batch> {
        let ids: Vec<String> = self.threads.keys().cloned().collect();
        ids.into_iter()
            .filter_map(|thread_id| self.take(&thread_id))
            .collect()
    }

    /// How many turns are pending in `thread_id`.
    #[must_use]
    pub fn pending(&self, thread_id: &str) -> usize {
        self.threads.get(thread_id).map_or(0, |p| p.turns.len())
    }

    fn take(&mut self, thread_id: &str) -> Option<Batch> {
        let pending = self.threads.remove(thread_id)?;
        if pending.turns.is_empty() {
            return None;
        }
        Some(Batch {
            thread_id: thread_id.to_string(),
            first: pending.first,
            turns: pending.turns,
        })
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
