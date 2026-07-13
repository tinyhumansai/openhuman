//! Compatibility wrapper for the tinycortex GitHub repository pipeline.

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::MemorySourceEntry;

pub use tinycortex::memory::sync::SyncOutcome;

pub async fn run_github_sync(
    source: &MemorySourceEntry,
    config: &Config,
) -> anyhow::Result<SyncOutcome> {
    tracing::debug!(
        source_id = %source.id,
        "[memory_sync:github] dispatching through tinycortex"
    );
    if crate::openhuman::memory::global::client_if_ready().is_none() {
        crate::openhuman::memory::global::init(config.workspace_dir.clone())
            .map_err(anyhow::Error::msg)?;
    }
    crate::openhuman::tinycortex::run_source_pipeline(source, config)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}
