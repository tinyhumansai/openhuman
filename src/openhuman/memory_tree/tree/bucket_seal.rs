//! Product adapters for tinycortex-owned bucket and document sealing.

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::trees::types::{Buffer, Tree};

pub use tinycortex::memory::tree::{LabelStrategy, LeafRef, MERGE_LEVEL_BASE};

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub async fn append_leaf(
    config: &Config,
    tree: &Tree,
    leaf: &LeafRef,
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    append_to_buffer(
        config,
        &tree.id,
        0,
        &leaf.chunk_id,
        leaf.token_count as i64,
        leaf.timestamp,
    )?;
    crate::openhuman::tinycortex::cascade_tree(config, tree, 0, false, strategy).await
}

pub fn append_leaf_deferred(config: &Config, tree: &Tree, leaf: &LeafRef) -> Result<bool> {
    tinycortex::memory::tree::append_leaf_deferred(&engine_config(config), tree, leaf)
}

pub fn append_to_buffer(
    config: &Config,
    tree_id: &str,
    level: u32,
    item_id: &str,
    token_delta: i64,
    item_ts: DateTime<Utc>,
) -> Result<()> {
    tinycortex::memory::tree::append_to_buffer(
        &engine_config(config),
        tree_id,
        level,
        item_id,
        token_delta,
        item_ts,
    )
}

pub async fn cascade_all_from(
    config: &Config,
    tree: &Tree,
    start_level: u32,
    force_now: Option<DateTime<Utc>>,
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    crate::openhuman::tinycortex::cascade_tree(
        config,
        tree,
        start_level,
        force_now.is_some(),
        strategy,
    )
    .await
}

pub async fn seal_document_subtree(
    config: &Config,
    tree: &Tree,
    doc_id: &str,
    version_ms: Option<i64>,
    chunk_ids: &[String],
    strategy: &LabelStrategy,
) -> Result<String> {
    crate::openhuman::tinycortex::seal_document_subtree(
        config, tree, doc_id, version_ms, chunk_ids, strategy,
    )
    .await
}

pub(crate) async fn seal_one_level(
    config: &Config,
    tree: &Tree,
    buffer: &Buffer,
    strategy: &LabelStrategy,
    enqueue_follow_ups: bool,
) -> Result<String> {
    crate::openhuman::tinycortex::seal_tree_level(
        config,
        tree,
        buffer,
        strategy,
        enqueue_follow_ups,
    )
    .await
}
