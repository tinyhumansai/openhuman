//! Compatibility exports for tinycortex summary-tree persistence types.

pub use tinycortex::memory::tree::store::{
    Buffer, EntityIndexStats, HotnessCounters, SummaryNode, Tree, TreeKind, TreeStatus,
    DEFAULT_FLUSH_AGE_SECS, INPUT_TOKEN_BUDGET, OUTPUT_TOKEN_BUDGET, SUMMARY_FANOUT,
    TOPIC_ARCHIVE_THRESHOLD, TOPIC_CREATION_THRESHOLD, TOPIC_RECHECK_EVERY,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_wire_discriminators_are_stable() {
        assert_eq!(TreeKind::parse("source").unwrap(), TreeKind::Source);
        assert_eq!(TreeStatus::parse("archived").unwrap(), TreeStatus::Archived);
    }

    #[test]
    fn tree_budgets_match_engine_defaults() {
        assert_eq!(INPUT_TOKEN_BUDGET, 50_000);
        assert_eq!(OUTPUT_TOKEN_BUDGET, 5_000);
        assert_eq!(SUMMARY_FANOUT, 10);
    }
}
