//! Workspace-backed conversation thread/message storage for the desktop UI —
//! thin host shim over `tinycortex::memory::conversations` (W7).
//!
//! Conversations are stored as JSONL files under the workspace (thread metadata
//! append-only in `threads.jsonl`; each thread's messages in a dedicated JSONL
//! file). The store / inverted-index / tokenizer / types engine is the crate's
//! (a byte-identical port, incl. the D1 rank-before-materialize fix); this
//! module re-exports that surface so the ~30 host consumers
//! (`openhuman::memory` re-exports it as `memory::conversations`, plus jsonrpc,
//! agent orchestration, agent_memory, threads, channels) keep their import paths
//! and identical `Result<_, String>` / on-disk behaviour unchanged.
//!
//! Host-retained: [`bus`] — the `core::event_bus` persistence subscriber that
//! bridges typed channel events onto the crate store (the crate abstracts the
//! bus behind its own `ConversationEventBus` trait; the host wires the real one).

mod bus;

pub use bus::register_conversation_persistence_subscriber;
pub use tinycortex::memory::conversations::{
    append_message, delete_thread, ensure_thread, get_messages, list_threads, purge_threads,
    update_message, update_thread_labels, update_thread_title, ConversationMessage,
    ConversationMessagePatch, ConversationPurgeStats, ConversationStore, ConversationThread,
    CreateConversationThread, CrossThreadHit,
};
