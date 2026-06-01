//! Tree persistence — source trees in `mem_tree_trees` keyed by [`TreeKind`].
//!
//! (The global/topic kinds were removed; their variants survive only as
//! inert serialization plumbing for the one-shot purge migration.) This
//! module hosts:
//! - `store`    — generic CRUD over the trees + summaries + buffers tables.
//! - `types`    — Tree, SummaryNode, TreeKind, TreeStatus, Buffer, and the
//!                entity-hotness types ([`HotnessCounters`], thresholds).
//! - `registry` — generic list / archive helpers.
//! - `hotness`  — entity-hotness side-table (now a read-only subconscious
//!                signal; the topic curator that wrote it was removed).
//!
//! Tree _logic_ (bucket_seal, flush, generic registry, source policy) stays
//! in `memory_tree`.

pub mod hotness;
pub mod registry;
pub mod store;
pub mod types;

pub use registry::{archive_tree, list_trees_by_kind};
pub use store::{get_summary_embedding, set_summary_embedding};
pub use types::{
    Buffer, EntityIndexStats, HotnessCounters, SummaryNode, Tree, TreeKind, TreeStatus,
    INPUT_TOKEN_BUDGET, OUTPUT_TOKEN_BUDGET, SUMMARY_FANOUT, TOPIC_ARCHIVE_THRESHOLD,
    TOPIC_CREATION_THRESHOLD, TOPIC_RECHECK_EVERY,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_module_reexports_expected_constants() {
        assert_eq!(INPUT_TOKEN_BUDGET, 50_000);
        assert_eq!(OUTPUT_TOKEN_BUDGET, 5_000);
        assert_eq!(SUMMARY_FANOUT, 10);
        assert!(TOPIC_CREATION_THRESHOLD > TOPIC_ARCHIVE_THRESHOLD);
        assert!(TOPIC_RECHECK_EVERY > 0);
    }
}
