//! Workspace-backed conversation thread/message storage — the implementation,
//! not the wiring.
//!
//! Conversations are stored as JSONL files under
//! `<workspace>/memory/conversations/`. Thread metadata is an append-only
//! upsert/delete log in `threads.jsonl`; each thread's messages live in a
//! dedicated JSONL file under `threads/<hex(thread_id)>.jsonl` for
//! straightforward inspection and recovery.
//!
//! This is **transcript persistence**, not semantic indexing: it owns the raw
//! thread/message records and a local trigram/CJK-bigram inverted index for
//! cross-thread substring search over those transcripts. The richer
//! summary-tree archival of the same transcripts lives in
//! [`crate::openhuman::memory::tree`].
//!
//! ## Layout
//!
//! - [`types`] — the on-disk wire types (threads, messages, patches, hits).
//! - [`tokenize`] — multilingual normalization + character n-gram tokenizer.
//! - [`inverted_index`] — in-memory trigram/bigram index over message content.
//! - [`store`] — the JSONL [`ConversationStore`] (append/read/update/delete,
//!   process-wide write serialization, warm-index cache, cross-thread search).
//!
//! The channel-persistence subscriber that mirrors inbound/processed channel
//! turns into this store is the host's own [`super::bus`], not part of this
//! subtree — see the note on the round trip below.
//!
//! # Why this code is here again (#5560)
//!
//! This store *originated* in OpenHuman as `memory_conversations`, was lifted
//! out into the memory engine, and is now back. The lift was never justified
//! by the engine: nothing in `tinycortex` ever called into this module, its
//! only imports are `std` + `async_trait`/`chrono`/`parking_lot`/`serde`/
//! `serde_json`/`uuid`, and its sole coupling to the rest of the engine was a
//! one-line `from_config` constructor. It was, in effect, host code parked in
//! a library, and it made up roughly half of this host's remaining
//! `tinycortex::` call sites. Bringing it home is what lets the host stop
//! compiling against that crate.
//!
//! ## What the round trip cost, and what did not come back
//!
//! The port out made four mechanical substitutions to avoid touching the
//! engine's `Cargo.toml`. All four are kept, and it is worth being explicit
//! that three of them are kept for a *different* reason than the one that
//! introduced them — `hex` and `tempfile` are both ordinary dependencies of
//! this crate, so nothing forces the substitutions any more:
//!
//! - `once_cell::sync::Lazy` → [`std::sync::LazyLock`] for the process-wide
//!   statics. Same semantics, and `LazyLock` is std now, so there is nothing
//!   to undo.
//! - `hex::encode` → the local `hex_encode` helper for per-thread filenames.
//!   This one function derives the path every existing transcript already
//!   lives at, so it stays byte-for-byte the code that wrote them.
//! - `tempfile::NamedTempFile` → write-to-temp + atomic-rename in
//!   `rewrite_jsonl`. Restoring `NamedTempFile` would change the temp file's
//!   name, its permissions and who deletes it on failure — a real change to
//!   the crash-safety path for a user's transcript, dressed as a cleanup.
//! - OpenHuman's `log`/`tracing` diagnostics were dropped because the engine
//!   had no logging facade. That one is a genuine loss rather than a wash, and
//!   re-adding the lines is still deliberately out of scope: it would mean
//!   edits inside the functions that write user transcripts, on a change whose
//!   entire claim is that nothing behavioural moved.
//!
//! Undoing any of the last three is a reasonable follow-up. It is a *separate*
//! follow-up, reviewed on its own merits, with the diff visible — not a
//! tidy-up smuggled into a move.
//!
//! The engine's `bus.rs` did not come back either. It existed only to abstract
//! the host's event bus behind a `ConversationEventBus` trait so the engine
//! could carry the subscriber without depending on the host's channel layer.
//! The host never used it — [`super::bus`] has always held the live subscriber,
//! wired directly to `core::bus::BUS` and to the real
//! `conversation_history_key`. With the store back home, the abstraction has
//! nothing left to abstract, so the host's version is simply repointed at the
//! local store and the engine's copy is left where it is.

mod inverted_index;
#[allow(clippy::module_inception)]
mod store;
mod tokenize;
mod types;

pub use store::{
    append_message, delete_thread, ensure_thread, get_messages, list_threads, purge_threads,
    update_message, update_thread_labels, update_thread_title, ConversationPurgeStats,
    ConversationStore,
};
pub use types::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
    CrossThreadHit,
};
