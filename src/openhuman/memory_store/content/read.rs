//! Product Config adapters over tinycortex content readers.

pub use tinycortex::memory::store::content::{
    read_chunk_file, read_summary_file, verify_chunk_file, verify_summary_file, ChunkFileContents,
    VerifyResult,
};

fn memory_config(config: &crate::openhuman::config::Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn read_chunk_body(
    config: &crate::openhuman::config::Config,
    chunk_id: &str,
) -> anyhow::Result<String> {
    tinycortex::memory::store::content::read_chunk_body(&memory_config(config), chunk_id)
}

pub fn read_summary_body(
    config: &crate::openhuman::config::Config,
    summary_id: &str,
) -> anyhow::Result<String> {
    tinycortex::memory::store::content::read_summary_body(&memory_config(config), summary_id)
}
