//! Backend-backed chat thread/message storage (`threads::store`).
//!
//! Conversations live in the process's storage backend
//! ([`crate::storage`]), through TinyAgents' `DriverConversationStore`: thread
//! metadata as one document per thread and each thread's messages as an
//! append stream. This is **transcript persistence**: the raw records plus
//! cross-thread substring search over them. It is not memory: memory v2
//! (`crate::memory`) only *ingests* committed turns through its own bus
//! subscriber.
//!
//! TinyAgents removed its path-bound JSONL store, so the host keeps only:
//!
//! - [`backend`] - the workspace-keyed synchronous facade
//!   (`ConversationStore::new(workspace_dir)` and the free functions) the ~50
//!   callers already use, resolving the backend and the conversation scope.
//! - `legacy_import` - the one-time, read-only import of the retired
//!   `<workspace>/memory/conversations` JSONL tree. A marker document records
//!   completion; the files are never modified.
//! - [`blocking`] - `spawn_blocking` wrappers. Every store entry point is
//!   synchronous and blocks on the storage bridge, so an `async fn` that calls
//!   one directly parks a tokio **worker** thread. Request paths must use these
//!   (#5156).
//! - `bus` - the `core::bus` subscriber that mirrors inbound and processed
//!   channel turns into the store, so Slack/Telegram/... persist alongside the
//!   UI's own threads.
//!
//! The two message types keep their historical host spelling
//! (`ConversationMessage`, `ConversationMessagePatch`) as aliases of
//! `ThreadMessage` / `ThreadMessagePatch`.

pub mod backend;
pub mod blocking;

mod bus;
mod legacy_import;
mod storage_schema;
pub use storage_schema::storage_schema;
#[cfg(test)]
#[path = "legacy_import_tests.rs"]
mod legacy_import_tests;
mod transcript_key;

use backend as store;
use tinyagents_session::threads as types;

/// Counts returned when the host purges stored conversations.
///
/// This keeps the host-facing store contract independent of the underlying
/// session-store crate's concrete return type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversationPurgeStats {
    /// Number of conversations removed.
    pub thread_count: usize,
    /// Number of messages removed across those conversations.
    pub message_count: usize,
}

impl From<types::ConversationPurgeStats> for ConversationPurgeStats {
    fn from(stats: types::ConversationPurgeStats) -> Self {
        Self {
            thread_count: stats.thread_count,
            message_count: stats.message_count,
        }
    }
}

// Only the hosted-channel relay (`channels`) claims turns.
#[cfg(feature = "channels")]
pub(crate) use bus::claim_channel_turn;
pub use bus::register_conversation_persistence_subscriber;
pub use store::{
    append_message, delete_messages_from, delete_thread, ensure_thread, list_threads,
    purge_threads, update_message, update_thread_labels, update_thread_title,
    update_thread_working_dir, ConversationStore,
};
pub(crate) use transcript_key::bind_transcript_key;
pub use types::{
    is_deterministic_message_id, reply_run_id, run_reply_message_id, ConversationThread,
    CreateConversationThread, CrossThreadHit, ThreadMessage as ConversationMessage,
    ThreadMessagePatch as ConversationMessagePatch,
};

/// A thread's messages without artifact rows. Artifact rows place artifact
/// cards in the UI and must never reach a model, so this is what every core
/// reader uses; only the UI's message list reads them
/// ([`blocking::get_messages_with_artifact_rows`]).
pub fn get_messages(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: std::path::PathBuf,
    thread_id: &str,
) -> Result<Vec<ConversationMessage>, String> {
    store::get_messages(ctx, workspace_dir, thread_id)
        .map(crate::threads::artifact_rows::without_artifact_rows)
}
