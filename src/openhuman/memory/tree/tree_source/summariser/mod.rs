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
use crate::openhuman::memory::tree::tree_source::types::TreeKind;

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
/// [`Config`]. The cloud-default refactor changed the resolution rules:
///
/// - `llm_backend = "cloud"` (default): always returns the LLM summariser
///   routed through the OpenHuman backend's `cloud_llm_model`
///   (defaulting to `summarization-v1`).
/// - `llm_backend = "local"`: returns the LLM summariser only when both
///   `llm_summariser_endpoint` AND `llm_summariser_model` are set;
///   otherwise returns the [`inert::InertSummariser`] fallback.
///
/// In all cases the LLM summariser itself soft-falls-back to inert per
/// seal on transport failure, so seal cascades never abort.
///
/// Returned as `Arc<dyn Summariser>` so the ingest pipeline can pass it
/// by reference to `append_leaf` and `route_leaf_to_topic_trees`
/// without threading a generic type parameter through every caller.
pub fn build_summariser(config: &Config) -> Arc<dyn Summariser> {
    use crate::openhuman::config::{LlmBackend, DEFAULT_CLOUD_LLM_MODEL};
    use crate::openhuman::memory::tree::chat::{build_chat_provider, ChatConsumer};

    // Resolve the model identifier to log alongside the provider name.
    // Returns None (→ inert fallback) only when llm_backend=local and the legacy
    // llm_summariser_endpoint/_model fields are not both set.
    let model: Option<String> = match config.memory_tree.llm_backend {
        LlmBackend::Cloud => Some(
            config
                .memory_tree
                .cloud_llm_model
                .clone()
                .unwrap_or_else(|| DEFAULT_CLOUD_LLM_MODEL.to_string()),
        ),
        LlmBackend::Local => {
            let endpoint = config
                .memory_tree
                .llm_summariser_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let m = config
                .memory_tree
                .llm_summariser_model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match (endpoint, m) {
                (Some(_), Some(m)) => Some(m.to_string()),
                _ => None,
            }
        }
    };

    let Some(model) = model else {
        log::debug!(
            "[tree_source::summariser] llm_summariser not configured for llm_backend={} \
             — using InertSummariser",
            config.memory_tree.llm_backend.as_str()
        );
        return Arc::new(inert::InertSummariser::new());
    };

    let provider = match build_chat_provider(config, ChatConsumer::Summarise) {
        Ok(p) => p,
        Err(err) => {
            log::warn!(
                "[tree_source::summariser] build_chat_provider failed: {err:#} — \
                 falling back to InertSummariser"
            );
            return Arc::new(inert::InertSummariser::new());
        }
    };

    log::info!(
        "[tree_source::summariser] using LlmSummariser provider={} model={}",
        provider.name(),
        model
    );
    Arc::new(llm::LlmSummariser::new(
        llm::LlmSummariserConfig { model },
        provider,
    ))
}
