//! Archivist — background PostTurnHook that extracts lessons, indexes
//! episodic records, and manages conversation segments with event extraction.
//!
//! After each turn, the Archivist:
//! 1. Inserts the turn into the FTS5 episodic table.
//! 2. Manages conversation segments (boundary detection + lifecycle).
//! 3. On segment close: produces an LLM recap (soft-fallback to heuristic),
//!    embeds the recap, extracts events, and updates user profile.
//! 4. Extracts simple lessons from tool failures.
//! 5. (Phase 2 / #566) At segment close/flush, ingests the segment's raw prose
//!    turns (user + assistant; tool-call JSON stripped) into the memory tree as
//!    `source_id = "conversations:agent"` when
//!    `config.learning.chat_to_tree_enabled` is true. The leaf is RAW PROSE —
//!    the LLM recap is NEVER fed into the tree (evidence-vs-interpretation
//!    policy). Each leaf carries episodic provenance stamped in `source_ref`.
//! 6. `flush_open_segment` force-closes the trailing open segment at session
//!    end so the last segment always gets a recap + embedding + tree ingest.

mod helpers;
mod hook_impl;
mod lifecycle;
mod recap;
#[cfg(test)]
mod test_constructors;
mod tree_ingest;
mod types;

pub use types::ArchivistHook;

#[cfg(test)]
pub(crate) use crate::openhuman::agent::hooks::PostTurnHook;
#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use crate::openhuman::memory_store::profile;
#[cfg(test)]
pub(crate) use helpers::extract_profile_key;
#[cfg(test)]
pub(crate) use parking_lot::Mutex;
#[cfg(test)]
pub(crate) use rusqlite::Connection;
#[cfg(test)]
pub(crate) use std::sync::Arc;

#[cfg(test)]
#[path = "../archivist_tests.rs"]
mod tests;
