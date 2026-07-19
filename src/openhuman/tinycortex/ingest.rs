//! Host adapters for tinycortex on-demand ingestion.

use rusqlite::Transaction;
use tinycortex::memory::ingest::{QueueJobSink, TreeJobSink};
use tinycortex::memory::score::extract::{LlmEntityExtractor, LlmExtractorConfig};
use tinycortex::memory::score::ScoringConfig;

use crate::openhuman::config::Config;

#[derive(Default)]
pub struct HostTreeJobSink;

impl HostTreeJobSink {
    pub fn new() -> Self {
        Self
    }
}

impl TreeJobSink for HostTreeJobSink {
    fn enqueue_extract_tx(
        &self,
        tx: &Transaction<'_>,
        chunk_id: &str,
        default_max_attempts: u32,
    ) -> anyhow::Result<bool> {
        tracing::trace!(
            chunk_id,
            default_max_attempts,
            "[memory:ingest] enqueue extract job in chunk transaction"
        );
        let enqueued = QueueJobSink
            .enqueue_extract_tx(tx, chunk_id, default_max_attempts)
            .inspect_err(|error| {
                tracing::error!(
                    chunk_id,
                    error = %error,
                    "[memory:ingest] enqueue extract job failed"
                );
            })?;
        tracing::trace!(
            chunk_id,
            enqueued,
            "[memory:ingest] enqueue extract job outcome (false = already queued)"
        );
        Ok(enqueued)
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
        HostTreeJobSink::new(),
        scoring_config(config),
    )
}
