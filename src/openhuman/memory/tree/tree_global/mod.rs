//! Phase 3b — Global Activity Digest tree (#709 umbrella, spec in
//! `docs/MEMORY_ARCHITECTURE_LLD.md`).
//!
//! The global tree is a **single cross-source recap structure**: one tree
//! per workspace, built end-of-day from the source trees' current roots so
//! a later question like "what did I do in the last 7 days?" can be
//! answered with one summary hop. Unlike source trees whose L0 holds raw
//! chunk leaves, the global tree's L0 already holds synthesised daily
//! summaries — each one a fold of the day's activity across every active
//! source tree.
//!
//! Level conventions (time-axis aligned, not token-driven):
//!   - L0 = one node per **day** (emitted by `end_of_day_digest`)
//!   - L1 = one node per **week** (~7 daily leaves)
//!   - L2 = one node per **month** (~4 weekly nodes)
//!   - L3 = one node per **year** (~12 monthly nodes)
//!
//! Reuses Phase 3a storage (`mem_tree_trees`, `mem_tree_summaries`,
//! `mem_tree_buffers` with `kind='global'`) and the `Summariser` trait.
//! The `InertSummariser` fallback is explicitly an honest stub — entities
//! and topics stay empty until an LLM-backed summariser lands.
//!
//! Public surface at Phase 3b:
//! - [`registry::get_or_create_global_tree`] — singleton (scope="global")
//! - [`digest::end_of_day_digest`] — build one L0 daily node, cascade-seal
//! - [`recap::recap`] — select the right level for a time window

pub mod digest;
pub mod recap;
pub mod registry;
pub mod seal;

pub use digest::{end_of_day_digest, DigestOutcome};
pub use recap::{recap, RecapOutput};
pub use registry::get_or_create_global_tree;

/// Number of L0 (daily) nodes that seal into one L1 (weekly) node.
pub const WEEKLY_SEAL_THRESHOLD: usize = 7;

/// Number of L1 (weekly) nodes that seal into one L2 (monthly) node.
/// ~4.35 weeks per month; we round down to seal monthly-ish when enough
/// weekly material accumulates.
pub const MONTHLY_SEAL_THRESHOLD: usize = 4;

/// Number of L2 (monthly) nodes that seal into one L3 (yearly) node.
pub const YEARLY_SEAL_THRESHOLD: usize = 12;

/// Literal scope used for the singleton global tree.
pub const GLOBAL_SCOPE: &str = "global";

/// Token budget passed into the summariser for global-tree seals. The
/// token-based seal trigger is disabled on the global tree (we use a
/// time/count trigger instead), so this is purely a ceiling on the
/// summariser's output length at each level.
pub const GLOBAL_TOKEN_BUDGET: u32 = 4_000;
