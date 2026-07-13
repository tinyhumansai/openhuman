//! `Config` adapters for tinycortex-owned markdown tree persistence.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::tree_runtime::types::{TreeNode, TreeStatus};

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn tree_dir(config: &Config, namespace: &str) -> PathBuf {
    tinycortex::memory::tree::runtime::store::tree_dir(&engine_config(config), namespace)
}

pub fn buffer_dir(config: &Config, namespace: &str) -> PathBuf {
    tinycortex::memory::tree::runtime::store::buffer_dir(&engine_config(config), namespace)
}

pub fn node_file_path(config: &Config, namespace: &str, node_id: &str) -> PathBuf {
    tinycortex::memory::tree::runtime::store::node_file_path(
        &engine_config(config),
        namespace,
        node_id,
    )
}

pub use tinycortex::memory::tree::runtime::store::{validate_namespace, validate_node_id};

pub fn write_node(config: &Config, node: &TreeNode) -> Result<()> {
    tinycortex::memory::tree::runtime::store::write_node(&engine_config(config), node)
}

pub fn read_node(config: &Config, namespace: &str, node_id: &str) -> Result<Option<TreeNode>> {
    tinycortex::memory::tree::runtime::store::read_node(&engine_config(config), namespace, node_id)
}

pub fn read_children(config: &Config, namespace: &str, parent_id: &str) -> Result<Vec<TreeNode>> {
    tinycortex::memory::tree::runtime::store::read_children(
        &engine_config(config),
        namespace,
        parent_id,
    )
}

pub fn read_ancestors(config: &Config, namespace: &str, node_id: &str) -> Result<Vec<TreeNode>> {
    tinycortex::memory::tree::runtime::store::read_ancestors(
        &engine_config(config),
        namespace,
        node_id,
    )
}

pub fn count_nodes(config: &Config, namespace: &str) -> Result<u64> {
    tinycortex::memory::tree::runtime::store::count_nodes(&engine_config(config), namespace)
}

pub fn get_tree_status(config: &Config, namespace: &str) -> Result<TreeStatus> {
    tinycortex::memory::tree::runtime::store::get_tree_status(&engine_config(config), namespace)
}

pub fn collect_root_summaries_with_caps(
    workspace_dir: &Path,
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<(String, String, DateTime<Utc>)> {
    tinycortex::memory::tree::runtime::store::collect_root_summaries_with_caps(
        workspace_dir,
        per_namespace_cap,
        total_cap,
    )
}

pub fn list_namespaces_with_root(config: &Config) -> Result<Vec<String>> {
    tinycortex::memory::tree::runtime::store::list_namespaces_with_root(&engine_config(config))
}

pub fn delete_tree(config: &Config, namespace: &str) -> Result<u64> {
    tinycortex::memory::tree::runtime::store::delete_tree(&engine_config(config), namespace)
}

pub fn buffer_write(
    config: &Config,
    namespace: &str,
    content: &str,
    ts: &DateTime<Utc>,
    metadata: Option<&Value>,
) -> Result<PathBuf> {
    tinycortex::memory::tree::runtime::store::buffer_write(
        &engine_config(config),
        namespace,
        content,
        ts,
        metadata,
    )
}

pub fn buffer_read(config: &Config, namespace: &str) -> Result<Vec<(String, String)>> {
    tinycortex::memory::tree::runtime::store::buffer_read(&engine_config(config), namespace)
}

pub fn buffer_delete(config: &Config, namespace: &str, filenames: &[String]) -> Result<()> {
    tinycortex::memory::tree::runtime::store::buffer_delete(
        &engine_config(config),
        namespace,
        filenames,
    )
}

pub fn buffer_drain(config: &Config, namespace: &str) -> Result<Vec<(String, String)>> {
    tinycortex::memory::tree::runtime::store::buffer_drain(&engine_config(config), namespace)
}

pub fn parse_node_markdown_pub(raw: &str, namespace: &str, node_id: &str) -> Result<TreeNode> {
    tinycortex::memory::tree::runtime::store::parse_node_markdown_pub(raw, namespace, node_id)
}
