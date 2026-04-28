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
use crate::openhuman::memory::tree::util::redact::redact_endpoint;

pub use extractor::{CompositeExtractor, EntityExtractor, RegexEntityExtractor};
pub use llm::{LlmEntityExtractor, LlmExtractorConfig};
pub use types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};

/// Build the extractor used by seal handlers to label new summary nodes.
///
/// Composition:
/// - regex extractor — always on, mechanical, near-zero cost
/// - LLM extractor with `emit_topics: true` — added when
///   `memory_tree.llm_extractor_endpoint` and `..._model` are both set
///
/// Differs from [`super::ScoringConfig::from_config`] (the chunk-admission
/// builder) in two ways: returns *just* an extractor (no thresholds /
/// weights / drop logic — none of which apply at seal time), and flips
/// `emit_topics` on so summaries surface thematic labels alongside
/// entities. Leaf-side scoring is unchanged.
pub fn build_summary_extractor(config: &Config) -> Arc<dyn EntityExtractor> {
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

    let (Some(endpoint), Some(model)) = (endpoint, model) else {
        log::debug!(
            "[memory_tree::extract] summary extractor: LLM not configured — using regex-only"
        );
        return Arc::new(CompositeExtractor::regex_only());
    };

    let timeout_ms = config
        .memory_tree
        .llm_extractor_timeout_ms
        .unwrap_or(15_000);

    let cfg = LlmExtractorConfig {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        timeout: std::time::Duration::from_millis(timeout_ms),
        emit_topics: true,
        ..LlmExtractorConfig::default()
    };

    match LlmEntityExtractor::new(cfg) {
        Ok(llm) => {
            // Drop to debug (diagnostic, not always-on) and redact the endpoint
            // so embedded credentials (e.g. api keys in URL) don't leak.
            log::debug!(
                "[memory_tree::extract] summary extractor: regex + LLM endpoint={} model={} \
                 timeout_ms={} emit_topics=true",
                redact_endpoint(endpoint),
                model,
                timeout_ms
            );
            Arc::new(CompositeExtractor::new(vec![
                Box::new(RegexEntityExtractor),
                Box::new(llm),
            ]))
        }
        Err(err) => {
            log::warn!(
                "[memory_tree::extract] summary extractor: LlmEntityExtractor construction \
                 failed: {err:#} — falling back to regex-only"
            );
            Arc::new(CompositeExtractor::regex_only())
        }
    }
}
