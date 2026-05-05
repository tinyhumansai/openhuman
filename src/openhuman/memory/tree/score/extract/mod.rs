//! Entity extraction (Phase 2 / #708).
//!
//! Exposes [`EntityExtractor`] as a pluggable interface and a default
//! [`CompositeExtractor`] that runs a chain of extractors and merges their
//! output. Phase 2 ships with the mechanical regex extractor only; semantic
//! NER (GLiNER / LLM) plugs in later without changing any call sites.

mod extractor;
pub mod llm;
pub mod regex;
pub mod types;

use std::sync::Arc;

use crate::openhuman::config::{Config, LlmBackend, DEFAULT_CLOUD_LLM_MODEL};
use crate::openhuman::memory::tree::chat::{build_chat_provider, ChatConsumer};

pub use extractor::{CompositeExtractor, EntityExtractor, RegexEntityExtractor};
pub use llm::{LlmEntityExtractor, LlmExtractorConfig};
pub use types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};

/// Build the extractor used by seal handlers to label new summary nodes.
///
/// Composition:
/// - regex extractor — always on, mechanical, near-zero cost
/// - LLM extractor with `emit_topics: true` — added when the LLM backend
///   is reachable. For `llm_backend = "cloud"` (default) that's always. For
///   `llm_backend = "local"` we still require `llm_extractor_endpoint` +
///   `_model` to be set (otherwise the legacy regex-only path stays).
///
/// Differs from [`super::ScoringConfig::from_config`] (the chunk-admission
/// builder) in two ways: returns *just* an extractor (no thresholds /
/// weights / drop logic — none of which apply at seal time), and flips
/// `emit_topics` on so summaries surface thematic labels alongside
/// entities. Leaf-side scoring is unchanged.
pub fn build_summary_extractor(config: &Config) -> Arc<dyn EntityExtractor> {
    let model = resolve_extractor_model(config);
    let Some(model) = model else {
        log::debug!(
            "[memory_tree::extract] summary extractor: LLM model not resolvable for \
             llm_backend={} — using regex-only",
            config.memory_tree.llm_backend.as_str()
        );
        return Arc::new(CompositeExtractor::regex_only());
    };

    let cfg = LlmExtractorConfig {
        model: model.clone(),
        emit_topics: true,
        ..LlmExtractorConfig::default()
    };

    let provider = match build_chat_provider(config, ChatConsumer::Extract) {
        Ok(p) => p,
        Err(err) => {
            log::warn!(
                "[memory_tree::extract] summary extractor: build_chat_provider failed: \
                 {err:#} — falling back to regex-only"
            );
            return Arc::new(CompositeExtractor::regex_only());
        }
    };

    log::debug!(
        "[memory_tree::extract] summary extractor: regex + LLM provider={} model={} \
         emit_topics=true",
        provider.name(),
        model
    );
    Arc::new(CompositeExtractor::new(vec![
        Box::new(RegexEntityExtractor),
        Box::new(LlmEntityExtractor::new(cfg, provider)),
    ]))
}

/// Resolve the model identifier the extractor's [`ChatProvider`] should
/// target, returning `None` when the configured backend can't be served:
///
/// - `Cloud`: always returns the configured `cloud_llm_model` or its
///   `summarization-v1` default.
/// - `Local`: returns `Some(model)` only when both
///   `llm_extractor_endpoint` AND `llm_extractor_model` are set —
///   otherwise the legacy regex-only path engages.
pub(super) fn resolve_extractor_model(config: &Config) -> Option<String> {
    match config.memory_tree.llm_backend {
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
                .llm_extractor_endpoint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let model = config
                .memory_tree
                .llm_extractor_model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match (endpoint, model) {
                (Some(_), Some(m)) => Some(m.to_string()),
                _ => None,
            }
        }
    }
}
