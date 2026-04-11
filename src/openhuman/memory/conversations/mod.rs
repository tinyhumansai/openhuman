//! Workspace-backed conversation thread/message storage for the desktop UI.
//!
//! Conversations are stored as JSONL files under `<workspace>/memory/conversations/`.
//! Thread metadata is append-only in `threads.jsonl`; each thread's messages live
//! in a dedicated JSONL file for straightforward inspection and recovery.

mod bus;
mod store;
mod types;

pub use bus::register_conversation_persistence_subscriber;
pub use store::{
    append_message, delete_thread, ensure_thread, get_messages, list_threads, purge_threads,
    update_message, ConversationPurgeStats, ConversationStore,
};
pub use types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
};
