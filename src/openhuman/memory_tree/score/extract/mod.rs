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

use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::build_chat_runtime;

pub use extractor::{CompositeExtractor, EntityExtractor, RegexEntityExtractor};
pub use llm::{LlmEntityExtractor, LlmExtractorConfig};
pub use types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};

/// Build the extractor used by seal handlers to label new summary nodes.
///
/// Composition:
/// - regex extractor — always on, mechanical, near-zero cost
/// - LLM extractor with `emit_topics: true` — added when the unified
///   summarization workload can be built from inference routing.
///
/// Differs from [`super::ScoringConfig::from_config`] (the chunk-admission
/// builder) in two ways: returns *just* an extractor (no thresholds /
/// weights / drop logic — none of which apply at seal time), and flips
/// `emit_topics` on so summaries surface thematic labels alongside
/// entities. Leaf-side scoring is unchanged.
pub fn build_summary_extractor(config: &Config) -> Arc<dyn EntityExtractor> {
    let (provider, model) = match build_chat_runtime(config) {
        Ok(runtime) => runtime,
        Err(err) => {
            log::warn!(
                "[memory_tree::extract] summary extractor: build_chat_runtime failed: \
                 {err:#} — falling back to regex-only"
            );
            return Arc::new(CompositeExtractor::regex_only());
        }
    };

    let cfg = LlmExtractorConfig {
        model: model.clone(),
        emit_topics: true,
        output_language: config.output_language.clone(),
        ..LlmExtractorConfig::default()
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
