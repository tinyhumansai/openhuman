//! Workspace-backed conversation thread/message storage for the desktop UI.
//!
//! Conversations are stored as JSONL files under `<workspace>/memory/conversations/`.
//! Thread metadata is append-only in `threads.jsonl`; each thread's messages live
//! in a dedicated JSONL file for straightforward inspection and recovery.
//!
//! This module was split out of `openhuman::memory` into the top-level
//! `openhuman::memory_conversations` namespace so the high-level memory policy
//! layer does not also own UI thread persistence. `openhuman::memory` re-exports
//! this module as `memory::conversations` during the migration.

mod bus;
mod inverted_index;
mod store;
mod tokenize;
mod types;

pub use bus::register_conversation_persistence_subscriber;
pub use store::{
    append_message, delete_thread, ensure_thread, get_messages, list_threads, purge_threads,
    update_message, update_thread_labels, update_thread_title, ConversationPurgeStats,
    ConversationStore,
};
pub use types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
    CrossThreadHit,
};
