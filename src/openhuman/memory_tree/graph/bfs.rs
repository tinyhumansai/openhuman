//! `Config` adapter for tinycortex-owned bounded graph traversal.

use anyhow::Result;

use crate::openhuman::config::Config;

pub use tinycortex::memory::graph::PairDistance;

pub fn pair_distances(
    config: &Config,
    entity_ids: &[String],
    max_h: u32,
) -> Result<Vec<PairDistance>> {
    tinycortex::memory::graph::pair_distances(
        &crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone()),
        entity_ids,
        max_h,
    )
}
