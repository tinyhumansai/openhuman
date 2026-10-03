//! Workspace-backed chat thread/message storage (`threads::store`).
//!
//! Conversations are JSONL files under `<workspace>/memory/conversations/`:
//! thread metadata as an append-only upsert/delete log in `threads.jsonl`, and
//! each thread's messages in a dedicated file under
//! `threads/<hex(thread_id)>.jsonl`. This is **transcript persistence** — the
//! raw records plus a trigram/CJK-bigram index for cross-thread substring
//! search over them. It is not memory: memory v2 (`crate::memory`) only
//! *ingests* committed turns through its own bus subscriber.
//!
//! The store itself (on-disk format, locking, warm index cache, CRUD and
//! search) is `tinyagents_session::threads`, re-exported below so callers
//! name `crate::threads::store::{...}`. The two message types keep their
//! historical host spelling (`ConversationMessage`, `ConversationMessagePatch`)
//! as aliases of `ThreadMessage` / `ThreadMessagePatch`; type names never
//! reach the disk. What stays here is host wiring:
//!
//! - [`blocking`] - `spawn_blocking` wrappers. Every store entry point is
//!   synchronous and can take `parking_lot` locks across fsync'd file IO, so an
//!   `async fn` that calls one directly parks a tokio **worker** thread for the
//!   whole wait. Request paths must use these (#5156).
//! - `bus` - the `core::bus` subscriber that mirrors inbound and processed
//!   channel turns into the store, so Slack/Telegram/... persist alongside the
//!   UI's own threads.
//!
//! The on-disk format is unchanged: the root is still
//! `<workspace>/memory/conversations`.

pub mod blocking;

mod bus;
use tinyagents_session::threads as store;

pub use bus::register_conversation_persistence_subscriber;
pub use store::{
    append_message, delete_messages_from, delete_thread, ensure_thread, get_messages,
    is_deterministic_message_id, list_threads, purge_threads, reply_run_id, run_reply_message_id,
    update_message, update_thread_labels, update_thread_title, ConversationPurgeStats,
    ConversationStore, ConversationThread, CreateConversationThread, CrossThreadHit,
    ThreadMessage as ConversationMessage, ThreadMessagePatch as ConversationMessagePatch,
};
