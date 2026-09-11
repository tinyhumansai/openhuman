//! Wire/storage types for the workspace-backed conversation store: threads,
//! messages, create requests, and partial-update patches.
//!
//! These mirror one-to-one the JSONL records persisted under
//! `<workspace>/memory/conversations/`. The `camelCase` serde renames and the
//! `type`/`extraMetadata` wire keys are part of the OpenHuman on-disk contract
//! and must be preserved byte-for-byte so existing transcripts keep loading.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A persisted conversation thread, mirroring one entry in `threads.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThread {
    /// Stable thread identifier; the JSONL key callers reference everywhere.
    pub id: String,
    /// Human-readable thread title.
    pub title: String,
    /// Optional host chat id (e.g. a messaging platform chat); omitted from the
    /// wire record when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    /// Whether the thread is currently active (not archived/closed).
    pub is_active: bool,
    /// Cached count of messages in the thread's log.
    pub message_count: usize,
    /// ISO-8601 timestamp of the most recent message.
    pub last_message_at: String,
    /// ISO-8601 timestamp of thread creation.
    pub created_at: String,
    /// Parent thread id when this thread was branched from another; omitted from
    /// the wire record when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    /// Free-form labels/tags; defaults to empty when absent on disk.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Optional personality id bound to the thread; omitted from the wire record
    /// when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_id: Option<String>,
}

/// A single message appended to a thread's JSONL log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    /// Stable message identifier, unique within the thread's log.
    pub id: String,
    /// Message body text.
    pub content: String,
    /// Message kind; serialized under the wire key `type` (e.g. `"text"`).
    #[serde(rename = "type")]
    pub message_type: String,
    /// Arbitrary per-message metadata; serialized under the wire key
    /// `extraMetadata`. Defaults to JSON null when absent.
    #[serde(default)]
    pub extra_metadata: Value,
    /// Sender identifier/role for the message.
    pub sender: String,
    /// ISO-8601 timestamp of when the message was created.
    pub created_at: String,
}

/// Input payload to create-or-update a thread via
/// [`ConversationStore::ensure_thread`](super::store::ConversationStore::ensure_thread).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConversationThread {
    /// Desired thread id; reused as-is if the thread already exists.
    pub id: String,
    /// Initial thread title.
    pub title: String,
    /// ISO-8601 creation timestamp to record.
    pub created_at: String,
    /// Optional parent thread id when branching from an existing thread.
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    /// Optional initial labels; `None` leaves the thread's labels unset.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Optional personality id to bind to the thread.
    #[serde(default)]
    pub personality_id: Option<String>,
}

/// Partial update to apply to a stored message (e.g. rewriting `extraMetadata`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessagePatch {
    /// Replacement `extraMetadata` payload; `None` leaves existing metadata
    /// untouched.
    #[serde(default)]
    pub extra_metadata: Option<Value>,
}

/// A single match returned by
/// [`ConversationStore::search_cross_thread_messages`](super::store::ConversationStore::search_cross_thread_messages).
/// Carries the source `thread_id` so the caller can render provenance into the
/// `[Cross-chat context]` block (issue #1505).
#[derive(Debug, Clone, PartialEq)]
pub struct CrossThreadHit {
    /// Source thread the match came from; drives provenance rendering.
    pub thread_id: String,
    /// Id of the matched message within that thread.
    pub message_id: String,
    /// Sender role of the matched message.
    pub role: String,
    /// Matched message content.
    pub content: String,
    /// ISO-8601 timestamp of the matched message.
    pub created_at: String,
    /// Relevance score for the match; higher is more relevant.
    pub score: f64,
}

/// Prefix of the message ids the core mints **deterministically** rather than
/// from a fresh UUID.
///
/// Only such an id can be presented to the store twice by two different
/// writers, so it is also the marker
/// [`is_deterministic_message_id`] keys the store's idempotency lookup on.
pub const DETERMINISTIC_MESSAGE_ID_PREFIX: &str = "agent:";

/// The id an autonomous run's closing reply is stored under.
///
/// Two writers legitimately persist that one reply — the core's
/// `task_session::append_final` and the client that also persists the
/// `chat_done` the run announces (`ChatRuntimeProvider` mirrors this shape for
/// `client_id: "system"` turns) — so both must derive the same id and the store
/// must collapse the second write onto the first (#5933).
pub fn run_reply_message_id(run_id: &str) -> String {
    format!("{DETERMINISTIC_MESSAGE_ID_PREFIX}{run_id}")
}

/// Whether `id` is one the core mints deterministically, i.e. one a second
/// writer can legitimately present again.
///
/// This is what buys back the constant-time append path: every other id in the
/// store is UUID-fresh by construction (`user:<uuid>`, `<sender>:<uuid>`), can
/// never be re-presented, and so must not pay for a duplicate lookup. The
/// `agent:<uuid>` ids the subagent/worker-thread writers mint do match — they
/// pay for a lookup they can never hit, which is one cheap scan of a
/// two-message worker transcript.
pub fn is_deterministic_message_id(id: &str) -> bool {
    id.starts_with(DETERMINISTIC_MESSAGE_ID_PREFIX)
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
