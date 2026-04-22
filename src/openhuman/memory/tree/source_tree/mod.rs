//! Phase 3a — summary trees + bucket-seal mechanics (#709).
//!
//! A thin orchestration layer on top of Phase 1 chunks and Phase 2 scores
//! that lifts individual leaves into a hierarchy of sealed summary nodes,
//! one tree per ingest source. See `docs/MEMORY_ARCHITECTURE_LLD.md` for
//! the full design. The module is isolated from the legacy
//! `openhuman::memory` layer and only depends on sibling `tree::*` modules.
//!
//! Public surface at Phase 3a:
//! - [`registry::get_or_create_source_tree`] — idempotent tree lookup
//! - [`bucket_seal::append_leaf`] — push a chunk into its tree, cascade-seal on budget
//! - [`flush::flush_stale_buffers`] — time-based seal of buffers that never cross budget
//! - [`summariser::inert::InertSummariser`] — deterministic fallback summariser
//!
//! Phases 3b / 3c will add `global` and `topic` trees; both reuse the
//! storage and cascade primitives defined here.

pub mod bucket_seal;
pub mod flush;
pub mod registry;
pub mod store;
pub mod summariser;
pub mod types;

pub use bucket_seal::{append_leaf, LeafRef};
pub use registry::get_or_create_source_tree;
pub use summariser::{inert::InertSummariser, Summariser};
pub use types::{Buffer, SummaryNode, Tree, TreeKind, TreeStatus, TOKEN_BUDGET};
