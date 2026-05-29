//! codegraph — content-addressed code retrieval for coding subagents.
//!
//! The seed engine behind the issue-crusher / pr-reviewer skills. Retrieval is
//! `BM25 (SQLite FTS5) ∪ structural-aug dense (embeddings domain)`, RRF-fused.
//! Indexing is content-addressed: every file's `{tokens, struct-doc embedding}`
//! is cached by its git **blob SHA** (+ embedding-model signature); a branch's
//! index is just its per-`(repo, ref)` **manifest** rows joined to the shared
//! blob cache at query time. Branch switch / new commit / pull only (re)embed
//! the blobs that actually changed.
//!
//! Pure Rust: `tree-sitter` for structure, `rusqlite`+FTS5 for lexical, and the
//! `embeddings` domain (cloud by default) for vectors. No Python, no extra
//! services.
//!
//! Layers:
//! - [`store`] — persistent SQLite blob cache + manifests (this commit).
//! - `index`  — tree-sitter extract + FTS5 + dense, incremental (next).
//! - `search` — BM25 ∪ dense RRF + coverage flag (next).

pub mod index;
pub mod search;
pub mod store;

pub use index::{
    code_tokens, count_code_files, current_ref, index_ref, structural_doc, IndexMode, IndexReport,
    LEXICAL_MODEL,
};
pub use search::{search_ref, Coverage, SearchOutcome};
pub use store::{BlobEntry, CodegraphStore};
