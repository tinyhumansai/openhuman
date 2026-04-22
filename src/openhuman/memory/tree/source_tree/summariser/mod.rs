//! Summariser trait + fallback (#709).
//!
//! A summariser folds N buffered items into one sealed summary. Phase 3a
//! ships an `InertSummariser` that concatenates the contributions and
//! truncates to the token budget — enough to make the tree mechanics
//! observable end-to-end without requiring an LLM. Real summarisation
//! (Ollama, etc.) can slot in by implementing the trait.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::openhuman::memory::tree::source_tree::types::TreeKind;

pub mod inert;

/// One contribution being folded — either a raw leaf (chunk) at L0→L1, or
/// a lower-level summary at L_n→L_{n+1}.
#[derive(Clone, Debug)]
pub struct SummaryInput {
    /// Primary key of the contribution (chunk id or summary id).
    pub id: String,
    pub content: String,
    pub token_count: u32,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub time_range_start: DateTime<Utc>,
    pub time_range_end: DateTime<Utc>,
    /// Score signal from scoring (for leaves) or parent seal (for summaries).
    pub score: f32,
}

/// Opaque context passed to the summariser — lets implementations log /
/// identify which tree is being sealed without threading config globally.
#[derive(Clone, Debug)]
pub struct SummaryContext<'a> {
    pub tree_id: &'a str,
    pub tree_kind: TreeKind,
    pub target_level: u32,
    pub token_budget: u32,
}

/// Output of a summariser invocation.
#[derive(Clone, Debug)]
pub struct SummaryOutput {
    pub content: String,
    pub token_count: u32,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
}

#[async_trait]
pub trait Summariser: Send + Sync {
    /// Fold the inputs into a single summary. `ctx.token_budget` is an
    /// upper bound on the produced `token_count`; implementations SHOULD
    /// stay well under it so parents have room to include this summary.
    async fn summarise(
        &self,
        inputs: &[SummaryInput],
        ctx: &SummaryContext<'_>,
    ) -> Result<SummaryOutput>;
}
