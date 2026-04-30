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

use std::sync::Arc;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::types::TreeKind;

pub mod inert;
pub mod llm;

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

/// Build the summariser implementation driven by the workspace's
/// [`Config`]. When `memory_tree.llm_summariser_endpoint` and
/// `llm_summariser_model` are both set, return the Ollama-backed
/// [`llm::LlmSummariser`] (which itself soft-falls-back to inert on
/// transport failure). Otherwise return [`inert::InertSummariser`].
///
/// Returned as `Arc<dyn Summariser>` so the ingest pipeline can pass it
/// by reference to `append_leaf` and `route_leaf_to_topic_trees`
/// without threading a generic type parameter through every caller.
pub fn build_summariser(config: &Config) -> Arc<dyn Summariser> {
    let endpoint = config
        .memory_tree
        .llm_summariser_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let model = config
        .memory_tree
        .llm_summariser_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (Some(endpoint), Some(model)) = (endpoint, model) else {
        log::debug!(
            "[source_tree::summariser] llm_summariser not configured — using InertSummariser"
        );
        return Arc::new(inert::InertSummariser::new());
    };

    // 120s default — matches `LlmSummariserConfig::default()`. Lets a
    // small/medium local model finish the seal-budget summary on a
    // cold-loaded weight cache without spurious timeouts.
    let timeout_ms = config
        .memory_tree
        .llm_summariser_timeout_ms
        .unwrap_or(120_000);

    let cfg = llm::LlmSummariserConfig {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        timeout: std::time::Duration::from_millis(timeout_ms),
    };
    match llm::LlmSummariser::new(cfg) {
        Ok(s) => {
            log::info!(
                "[source_tree::summariser] using LlmSummariser endpoint={} model={} timeout_ms={}",
                endpoint,
                model,
                timeout_ms
            );
            Arc::new(s)
        }
        Err(err) => {
            log::warn!(
                "[source_tree::summariser] LlmSummariser construction failed: {err:#} — \
                 falling back to InertSummariser"
            );
            Arc::new(inert::InertSummariser::new())
        }
    }
}
