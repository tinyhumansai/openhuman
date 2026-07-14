//! `Config` and transaction adapters for tinycortex tree persistence.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, Transaction};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::content::StagedSummary;
use crate::openhuman::memory_store::trees::types::{Buffer, SummaryNode, Tree, TreeKind};

pub(crate) use tinycortex::memory::tree::store::TreeCascadeDeletion;

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn insert_tree(config: &Config, tree: &Tree) -> Result<()> {
    tinycortex::memory::tree::store::insert_tree(&engine_config(config), tree)
}

pub(crate) fn insert_tree_conn(conn: &Connection, tree: &Tree) -> Result<()> {
    tinycortex::memory::tree::store::insert_tree_conn(conn, tree)
}

pub(crate) fn delete_tree_cascade_tx(
    tx: &Transaction<'_>,
    tree_id: &str,
) -> Result<TreeCascadeDeletion> {
    tinycortex::memory::tree::store::delete_tree_cascade_tx(tx, tree_id)
}

pub fn get_tree_by_scope(config: &Config, kind: TreeKind, scope: &str) -> Result<Option<Tree>> {
    tinycortex::memory::tree::store::get_tree_by_scope(&engine_config(config), kind, scope)
}

pub(crate) fn get_tree_by_scope_conn(
    conn: &Connection,
    kind: TreeKind,
    scope: &str,
) -> Result<Option<Tree>> {
    tinycortex::memory::tree::store::get_tree_by_scope_conn(conn, kind, scope)
}

pub fn get_tree(config: &Config, id: &str) -> Result<Option<Tree>> {
    tinycortex::memory::tree::store::get_tree(&engine_config(config), id)
}

pub fn get_trees_batch(config: &Config, ids: &[String]) -> Result<HashMap<String, Tree>> {
    tinycortex::memory::tree::store::get_trees_batch(&engine_config(config), ids)
}

pub fn list_trees_by_kind(config: &Config, kind: TreeKind) -> Result<Vec<Tree>> {
    tinycortex::memory::tree::store::list_trees_by_kind(&engine_config(config), kind)
}

pub(crate) fn update_tree_after_seal_tx(
    tx: &Transaction<'_>,
    tree_id: &str,
    root_id: &str,
    max_level: u32,
    sealed_at: DateTime<Utc>,
) -> Result<()> {
    tinycortex::memory::tree::store::update_tree_after_seal_tx(
        tx, tree_id, root_id, max_level, sealed_at,
    )
}

pub(crate) fn insert_summary_tx(
    tx: &Transaction<'_>,
    node: &SummaryNode,
    staged: Option<&StagedSummary>,
    model_signature: &str,
) -> Result<()> {
    tinycortex::memory::tree::store::insert_staged_summary_tx(tx, node, staged, model_signature)
}

pub fn set_summary_embedding(
    config: &Config,
    summary_id: &str,
    embedding: &[f32],
) -> Result<usize> {
    tinycortex::memory::tree::store::set_summary_embedding(
        &engine_config(config),
        summary_id,
        embedding,
    )?;
    Ok(1)
}

pub fn get_summary_embedding(config: &Config, summary_id: &str) -> Result<Option<Vec<f32>>> {
    tinycortex::memory::tree::store::get_summary_embedding(&engine_config(config), summary_id)
}

pub fn set_summary_embedding_for_signature(
    config: &Config,
    summary_id: &str,
    signature: &str,
    embedding: &[f32],
) -> Result<()> {
    tinycortex::memory::tree::store::set_summary_embedding_for_signature(
        &engine_config(config),
        summary_id,
        signature,
        embedding,
    )
}

pub fn mark_summary_reembed_skipped(
    config: &Config,
    summary_id: &str,
    signature: &str,
    reason: &str,
) -> Result<()> {
    tinycortex::memory::chunks::mark_summary_reembed_skipped(
        &engine_config(config),
        summary_id,
        signature,
        reason,
    )
}

pub fn clear_summary_reembed_skipped(
    config: &Config,
    summary_id: &str,
    signature: &str,
) -> Result<()> {
    tinycortex::memory::chunks::clear_summary_reembed_skipped(
        &engine_config(config),
        summary_id,
        signature,
    )
}

pub(crate) fn set_summary_embedding_for_signature_tx(
    tx: &Transaction<'_>,
    summary_id: &str,
    signature: &str,
    embedding: &[f32],
) -> Result<()> {
    tinycortex::memory::chunks::set_summary_embedding_for_signature_tx(
        tx, summary_id, signature, embedding,
    )
}

pub fn get_summary_embedding_for_signature(
    config: &Config,
    summary_id: &str,
    signature: &str,
) -> Result<Option<Vec<f32>>> {
    tinycortex::memory::tree::store::get_summary_embedding_for_signature(
        &engine_config(config),
        summary_id,
        signature,
    )
}

pub fn get_summary_embeddings_for_signature_batch(
    config: &Config,
    ids: &[String],
    signature: &str,
) -> Result<HashMap<String, Vec<f32>>> {
    tinycortex::memory::tree::store::get_summary_embeddings_for_signature_batch(
        &engine_config(config),
        ids,
        signature,
    )
}

pub fn get_summary_embeddings_batch(
    config: &Config,
    ids: &[String],
) -> Result<HashMap<String, Vec<f32>>> {
    tinycortex::memory::tree::store::get_summary_embeddings_batch(&engine_config(config), ids)
}

pub fn get_summary(config: &Config, id: &str) -> Result<Option<SummaryNode>> {
    tinycortex::memory::tree::store::get_summary(&engine_config(config), id)
}

pub fn get_summaries_batch(
    config: &Config,
    ids: &[String],
) -> Result<HashMap<String, SummaryNode>> {
    tinycortex::memory::tree::store::get_summaries_batch(&engine_config(config), ids)
}

pub fn list_summaries_at_level(
    config: &Config,
    tree_id: &str,
    level: u32,
) -> Result<Vec<SummaryNode>> {
    tinycortex::memory::tree::store::list_summaries_at_level(&engine_config(config), tree_id, level)
}

pub fn list_summaries_in_window(
    config: &Config,
    tree_id: &str,
    since_ms: i64,
    until_ms: i64,
) -> Result<Vec<SummaryNode>> {
    tinycortex::memory::tree::store::list_summaries_in_window(
        &engine_config(config),
        tree_id,
        since_ms,
        until_ms,
    )
}

pub fn count_summaries(config: &Config, tree_id: &str) -> Result<u64> {
    tinycortex::memory::tree::store::count_summaries(&engine_config(config), tree_id)
}

pub fn get_buffer(config: &Config, tree_id: &str, level: u32) -> Result<Buffer> {
    tinycortex::memory::tree::store::get_buffer(&engine_config(config), tree_id, level)
}

pub(crate) fn get_buffer_conn(conn: &Connection, tree_id: &str, level: u32) -> Result<Buffer> {
    tinycortex::memory::tree::store::get_buffer_conn(conn, tree_id, level)
}

pub(crate) fn upsert_buffer_tx(tx: &Transaction<'_>, buffer: &Buffer) -> Result<()> {
    tinycortex::memory::tree::store::upsert_buffer_tx(tx, buffer)
}

pub(crate) fn clear_buffer_tx(tx: &Transaction<'_>, tree_id: &str, level: u32) -> Result<()> {
    tinycortex::memory::tree::store::clear_buffer_tx(tx, tree_id, level)
}

pub fn list_stale_buffers(config: &Config, older_than: DateTime<Utc>) -> Result<Vec<Buffer>> {
    tinycortex::memory::tree::store::list_stale_buffers(&engine_config(config), older_than)
}
