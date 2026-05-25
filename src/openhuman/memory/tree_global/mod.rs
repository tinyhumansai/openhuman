//! Global tree instance — policy and orchestration for the singleton
//! cross-source digest tree.
//!
//! The generic tree engine lives in [`memory_tree`]; this module owns
//! the global-specific algorithms: end-of-day digest, window-scoped
//! recap, and count-based cascade-seal thresholds.

pub mod digest;
pub mod recap;
pub mod seal;

pub use crate::openhuman::memory_store::trees::get_or_create_global_tree;
pub use crate::openhuman::memory_store::trees::registry;
pub use crate::openhuman::memory_tree::tree::factory::GLOBAL_SCOPE;
use crate::openhuman::memory_tree::tree::TreeFactory;
pub use digest::{end_of_day_digest, DigestOutcome};
pub use recap::{recap, RecapOutput};

/// Number of L0 (daily) nodes that seal into one L1 (weekly) node.
pub const WEEKLY_SEAL_THRESHOLD: usize = 7;
/// Number of L1 (weekly) nodes that seal into one L2 (monthly) node.
pub const MONTHLY_SEAL_THRESHOLD: usize = 4;
/// Number of L2 (monthly) nodes that seal into one L3 (yearly) node.
pub const YEARLY_SEAL_THRESHOLD: usize = 12;
/// Token budget passed into the summariser for global-tree seals.
pub const GLOBAL_TOKEN_BUDGET: u32 = 4_000;

/// Canonical factory for the singleton global tree.
pub fn factory() -> TreeFactory<'static> {
    TreeFactory::global()
}
