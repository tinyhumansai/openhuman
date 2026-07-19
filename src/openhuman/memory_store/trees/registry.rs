//! `Config` adapters for tinycortex's tree registry.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::trees::types::{Tree, TreeKind};

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn list_trees_by_kind(config: &Config, kind: TreeKind) -> Result<Vec<Tree>> {
    tinycortex::memory::tree::store::list_trees_by_kind(&engine_config(config), kind)
}

pub fn archive_tree(config: &Config, tree_id: &str) -> Result<()> {
    log::debug!("[memory:trees] archive tree_id={tree_id}");
    tinycortex::memory::tree::store::archive_tree(&engine_config(config), tree_id)
}
