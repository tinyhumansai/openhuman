//! OpenHuman configuration and LLM wrappers for tinycortex raw rebuilds.

use crate::openhuman::config::Config;

pub use tinycortex::memory::sync::{RawCoverage, RawFileRef, RebuildOutcome};

pub fn raw_coverage(
    config: &Config,
    tree_scope: &str,
    archive_source_id: &str,
) -> anyhow::Result<RawCoverage> {
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    tinycortex::memory::sync::raw_coverage(&memory_config, tree_scope, archive_source_id)
}

pub fn needs_rebuild(config: &Config, tree_scope: &str, archive_source_id: &str) -> bool {
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    tinycortex::memory::sync::needs_rebuild(&memory_config, tree_scope, archive_source_id)
}

pub async fn rebuild_tree_from_raw(
    config: &Config,
    tree_scope: &str,
    archive_source_id: &str,
) -> anyhow::Result<RebuildOutcome> {
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    let summariser = crate::openhuman::tinycortex::HostSummariser::new(config.clone());
    tinycortex::memory::sync::rebuild_tree_from_raw(
        &memory_config,
        tree_scope,
        archive_source_id,
        &summariser,
    )
    .await
}
