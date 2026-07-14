//! OpenHuman configuration wrappers for tinycortex sync audit ownership.

use crate::openhuman::config::Config;

pub use tinycortex::memory::sync::{RealCostAccumulator, SyncAuditEntry};

pub fn append_audit_entry(config: &Config, entry: &SyncAuditEntry) {
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    if let Err(error) = tinycortex::memory::sync::append_audit_entry(&memory_config, entry) {
        tracing::warn!(%error, "[memory_sync:audit] tinycortex append failed");
    }
}

pub fn read_audit_log(config: &Config) -> Vec<SyncAuditEntry> {
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    match tinycortex::memory::sync::read_audit_log(&memory_config) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, "[memory_sync:audit] tinycortex read failed");
            Vec::new()
        }
    }
}

pub fn estimate_cost_usd(input_tokens: u64, output_tokens: u64) -> f64 {
    tinycortex::memory::sync::estimate_cost_usd(input_tokens, output_tokens)
}
