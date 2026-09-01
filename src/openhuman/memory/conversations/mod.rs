//! Workspace-backed conversation thread/message storage — the whole thing,
//! store and wiring both.
//!
//! Conversations are JSONL files under `<workspace>/memory/conversations/`:
//! thread metadata as an append-only upsert/delete log in `threads.jsonl`, and
//! each thread's messages in a dedicated file under
//! `threads/<hex(thread_id)>.jsonl`. This is **transcript persistence** — the
//! raw records plus a trigram/CJK-bigram index for cross-thread substring
//! search over them. The summary-tree archival of the same transcripts is
//! [`crate::openhuman::memory::tree`], a different index answering a different
//! question.
//!
//! Three parts, and the split is about ownership rather than size:
//!
//! - `store` — the implementation: on-disk format, the process-wide write
//!   lock, the warm index cache, CRUD and search. Everything below is
//!   re-exported from here, so callers name
//!   `crate::openhuman::memory::conversations::{…}` and never the subtree.
//! - [`blocking`] — `spawn_blocking` wrappers. Every store entry point is
//!   synchronous and takes a `parking_lot` mutex across fsync'd file IO, so an
//!   `async fn` that calls one directly parks a tokio **worker** thread for the
//!   whole wait. Request paths must use these (#5156).
//! - `bus` — the `core::bus` subscriber that mirrors inbound and processed
//!   channel turns into the store, so Slack/Telegram/… persist alongside the
//!   UI's own threads.
//!
//! # What #5560 changed here
//!
//! Until this change, `store` was `tinycortex::memory::conversations` and
//! roughly a dozen files across `threads`, `channels` and `agent` named that
//! crate directly — 72 of this host's ~156 `tinycortex::` call sites, the
//! single largest block of them. The store is back in the host now, and those
//! call sites name this module instead.
//!
//! It came back because the lift was never load-bearing: nothing inside the
//! engine ever called into this code, its imports are std plus
//! `async_trait`/`chrono`/`parking_lot`/`serde`/`serde_json`/`uuid`, and its
//! sole tie to the rest of the engine was a one-line `from_config` constructor
//! that no host caller used. The engine's own module docs recorded the code as
//! "ported from OpenHuman", which is the tell: it was host code parked in a
//! library. `store`'s module docs carry the full accounting of what the
//! round trip cost and what deliberately did not come back with it.
//!
//! Nothing about the on-disk format moved. The root is still
//! `<workspace>/memory/conversations`, derived by the same line of code from
//! the same argument every caller was already passing, so an existing
//! installation reads and writes exactly the files it did before.

pub mod blocking;

mod bus;
mod store;

pub use bus::register_conversation_persistence_subscriber;
pub use store::{
    append_message, delete_thread, ensure_thread, get_messages, list_threads, purge_threads,
    update_message, update_thread_labels, update_thread_title, ConversationMessage,
    ConversationMessagePatch, ConversationPurgeStats, ConversationStore, ConversationThread,
    CreateConversationThread, CrossThreadHit,
};
