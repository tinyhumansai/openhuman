//! Product adapters over tinycortex scoring and admission.

pub mod embed;
pub mod extract;
pub mod store;

use std::sync::Arc;

pub use anyhow::Result;
pub use tinycortex::memory::score::{
    persist_score_tx, score_chunk, score_chunks, score_chunks_fast, ScoreResult, ScoringConfig,
    DEFAULT_DEFINITE_DROP, DEFAULT_DEFINITE_KEEP, DEFAULT_DROP_THRESHOLD, PRIORITY_BOOST,
    PRIORITY_TAG,
};
pub use tinycortex::memory::score::{resolver, signals};

/// Build crate scoring policy from product inference routing.
pub fn scoring_config_from(config: &crate::openhuman::config::Config) -> ScoringConfig {
    let (provider, model) = match crate::openhuman::memory::chat::build_chat_runtime(config) {
        Ok((provider, model)) => (
            Arc::new(crate::openhuman::tinycortex::SeamChatProvider::new(
                provider,
            )) as Arc<dyn tinycortex::memory::score::extract::ChatProvider>,
            model,
        ),
        Err(error) => {
            log::warn!(
                "[memory::score] chat provider unavailable; using regex-only scoring: {error:#}"
            );
            return ScoringConfig::default_regex_only();
        }
    };
    let extractor = tinycortex::memory::score::extract::LlmEntityExtractor::new(
        tinycortex::memory::score::extract::LlmExtractorConfig {
            model,
            output_language: config.output_language.clone(),
            ..Default::default()
        },
        provider,
    );
    ScoringConfig::with_llm_extractor(Arc::new(extractor))
}
