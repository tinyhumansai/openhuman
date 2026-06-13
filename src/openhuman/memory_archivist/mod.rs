//! Memory archivist — chat conversation → tree.
//!
//! The archivist's one job is to take a chat conversation, strip the
//! noisy tool-call payloads from it, and push the resulting text into a
//! memory tree as a single leaf. The tree owns persistence + retrieval
//! from there on.
//!
//! ## Flow
//!
//! ```text
//!   Vec<Turn>          (raw conversation, tool calls included)
//!         │
//!         ▼
//!   clip::clean()      (strip tool_calls_json; keep role + content)
//!         │
//!         ▼
//!   compose::md()      (one md blob: ## role\n<content>\n\n... per turn)
//!         │
//!         ▼
//!   memory_tree::TreeWriteRequest
//!         │
//!         ▼
//!   memory_store::trees                 (append_leaf + cascade seal)
//! ```
//!
//! ## API
//!
//! - [`Turn`] — input shape, one per role/content/tool_calls record.
//! - [`clean_conversation`] — pure transform; returns a `Vec<Turn>` with
//!   tool-call payloads dropped.
//! - [`compose_conversation_md`] — pure transform; returns the markdown
//!   blob that will become a single tree leaf.
//! - [`archive_to_tree`] — end-to-end: clean → compose → append leaf to
//!   the named tree via `memory_tree`.
//!
//! ## Why "clip"?
//!
//! Tool-call JSON is verbose, model-specific, and rarely meaningful out
//! of context. Stripping it before the conversation lands in the tree
//! keeps summaries focused on natural-language content and keeps the
//! vector embedding signal clean.

pub mod clip;
pub mod compose;
pub mod store;
pub mod tree_writer;
pub mod types;

pub use clip::clean_conversation;
pub use compose::compose_conversation_md;
pub use store::{record_turn, session_entries};
pub use tree_writer::archive_to_tree;
pub use types::{ArchivedTurn, Turn};
