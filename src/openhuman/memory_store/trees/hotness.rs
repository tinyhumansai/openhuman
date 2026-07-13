//! `Config` adapters for tinycortex entity-hotness persistence.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::trees::types::HotnessCounters;

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn get(config: &Config, entity_id: &str) -> Result<Option<HotnessCounters>> {
    tinycortex::memory::tree::store::hotness::get(&engine_config(config), entity_id)
}

pub fn get_or_fresh(config: &Config, entity_id: &str) -> Result<HotnessCounters> {
    tinycortex::memory::tree::store::hotness::get_or_fresh(&engine_config(config), entity_id)
}

pub fn upsert(config: &Config, counters: &HotnessCounters) -> Result<()> {
    tinycortex::memory::tree::store::hotness::upsert(&engine_config(config), counters)
}

pub fn distinct_sources_for(config: &Config, entity_id: &str) -> Result<u32> {
    tinycortex::memory::tree::store::hotness::distinct_sources_for(
        &engine_config(config),
        entity_id,
    )
}

pub fn count(config: &Config) -> Result<u64> {
    tinycortex::memory::tree::store::hotness::count(&engine_config(config))
}
