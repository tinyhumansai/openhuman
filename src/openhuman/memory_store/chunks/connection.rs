//! `Config` adapters for tinycortex's chunk connection and recovery manager.

use anyhow::Result;
use rusqlite::Connection;

use crate::openhuman::config::Config;

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

#[doc(hidden)]
pub fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    tinycortex::memory::chunks::with_connection(&engine_config(config), f)
}

pub(crate) fn recover_corrupt_db(config: &Config) -> Result<bool> {
    log::warn!("[memory:chunks] checking corrupt database recovery");
    tinycortex::memory::chunks::recover_corrupt_db(&engine_config(config))
}

#[cfg(test)]
pub(crate) use tinycortex::memory::chunks::{is_transient_cold_start, try_cleanup_stale_files};
