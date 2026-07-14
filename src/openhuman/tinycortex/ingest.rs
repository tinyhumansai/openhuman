//! Host adapters for tinycortex on-demand ingestion.

use tinycortex::memory::ingest::TreeJobSink;
use tinycortex::memory::score::extract::{LlmEntityExtractor, LlmExtractorConfig};
use tinycortex::memory::score::ScoringConfig;

use crate::openhuman::config::Config;
use crate::openhuman::memory_queue::{self, ExtractChunkPayload, NewJob};

pub struct HostTreeJobSink {
    config: Config,
}

impl HostTreeJobSink {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl TreeJobSink for HostTreeJobSink {
    fn enqueue_extract(&self, chunk_id: &str) -> anyhow::Result<()> {
        let job = NewJob::extract_chunk(&ExtractChunkPayload {
            chunk_id: chunk_id.into(),
        })?;
        memory_queue::enqueue(&self.config, &job)?;
        Ok(())
    }
}

fn scoring_config(config: &Config) -> ScoringConfig {
    match super::build_chat_provider(config) {
        Ok(provider) => {
            let mut extractor = LlmExtractorConfig::default();
            extractor.output_language = config.output_language.clone();
            ScoringConfig::with_llm_extractor(std::sync::Arc::new(LlmEntityExtractor::new(
                extractor, provider,
            )))
        }
        Err(error) => {
            tracing::warn!(%error, "[memory:ingest] chat provider unavailable; using regex scoring");
            ScoringConfig::default_regex_only()
        }
    }
}

pub fn context(
    config: &Config,
) -> (
    tinycortex::memory::MemoryConfig,
    HostTreeJobSink,
    ScoringConfig,
) {
    (
        super::memory_config_from(config, config.workspace_dir.clone()),
        HostTreeJobSink::new(config.clone()),
        scoring_config(config),
    )
}
