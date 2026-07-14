//! `Config` adapters for tinycortex embedding sidecars and tombstones.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{Connection, Transaction};

use crate::openhuman::config::Config;

#[cfg(test)]
pub use tinycortex::memory::chunks::{embedding_to_blob, REEMBED_SKIP_KEY_MAX_LEN};

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub(crate) fn tree_active_signature(config: &Config) -> String {
    tinycortex::memory::chunks::tree_active_signature(&engine_config(config))
}

pub fn set_chunk_embedding(config: &Config, id: &str, embedding: &[f32]) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_embedding(&engine_config(config), id, embedding)
}

pub fn set_chunk_embedding_for_signature(
    config: &Config,
    id: &str,
    signature: &str,
    embedding: &[f32],
) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_embedding_for_signature(
        &engine_config(config),
        id,
        signature,
        embedding,
    )
}

pub(crate) fn has_uncovered_reembed_work(
    conn: &Connection,
    signature: &str,
) -> rusqlite::Result<bool> {
    tinycortex::memory::chunks::has_uncovered_reembed_work(conn, signature)
}

pub fn mark_chunk_reembed_skipped(
    config: &Config,
    id: &str,
    signature: &str,
    reason: &str,
) -> Result<()> {
    tinycortex::memory::chunks::mark_chunk_reembed_skipped(
        &engine_config(config),
        id,
        signature,
        reason,
    )
}

pub fn clear_chunk_reembed_skipped(config: &Config, id: &str, signature: &str) -> Result<()> {
    tinycortex::memory::chunks::clear_chunk_reembed_skipped(&engine_config(config), id, signature)
}

pub fn clear_reembed_skipped_for_signature(config: &Config, signature: &str) -> Result<usize> {
    tinycortex::memory::chunks::clear_reembed_skipped_for_signature(
        &engine_config(config),
        signature,
    )
}

pub(crate) fn set_chunk_embedding_for_signature_tx(
    tx: &Transaction<'_>,
    id: &str,
    signature: &str,
    embedding: &[f32],
) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_embedding_for_signature_tx(tx, id, signature, embedding)
}

pub fn get_chunk_embedding_for_signature(
    config: &Config,
    id: &str,
    signature: &str,
) -> Result<Option<Vec<f32>>> {
    tinycortex::memory::chunks::get_chunk_embedding_for_signature(
        &engine_config(config),
        id,
        signature,
    )
}

pub fn get_chunk_embedding(config: &Config, id: &str) -> Result<Option<Vec<f32>>> {
    tinycortex::memory::chunks::get_chunk_embedding(&engine_config(config), id)
}

pub fn get_chunk_embeddings_for_signature_batch(
    config: &Config,
    ids: &[String],
    signature: &str,
) -> Result<HashMap<String, Vec<f32>>> {
    tinycortex::memory::chunks::get_chunk_embeddings_for_signature_batch(
        &engine_config(config),
        ids,
        signature,
    )
}

pub fn get_chunk_embeddings_batch(
    config: &Config,
    ids: &[String],
) -> Result<HashMap<String, Vec<f32>>> {
    tinycortex::memory::chunks::get_chunk_embeddings_batch(&engine_config(config), ids)
}
