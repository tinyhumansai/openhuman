//! Raw-archive pointers and content-pointer accessors for chunk/summary rows.
//!
//! `RawRef` lets ingest pipelines mirror full message bodies to on-disk
//! archives under `<content_root>/raw/` while storing only a ≤500-char
//! preview in the SQLite `content` column. Retrieval reads the archive
//! directly instead of going through the SQL preview path.
//!
//! **W3 sub-store flip:** these operations now delegate to
//! [`tinycortex::memory::chunks`] (ported from this exact module — identical SQL
//! against the same `mem_tree_chunks` / `mem_tree_summaries` tables in the shared
//! `chunks.db` the crate now owns). The host signatures are preserved so the ~4
//! external callers (`content::read`, memory_sync gmail/slack ingest, rebuild)
//! are untouched; only `&Config` is mapped to the crate's `MemoryConfig`.

use anyhow::Result;
use rusqlite::Transaction;

use crate::openhuman::config::Config;
use crate::openhuman::tinycortex::memory_config_from;

// `RawRef` is re-exported from the crate (identical fields + serde derives), so
// every `chunks::RawRef { path, start, end }` construction site keeps compiling.
pub use tinycortex::memory::chunks::RawRef;

/// Map the host `Config` to the engine `MemoryConfig` addressing the same
/// `<workspace_dir>/memory_tree/chunks.db` (only `workspace` is load-bearing for
/// these DB ops).
fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    memory_config_from(config, config.workspace_dir.clone())
}

/// Stash a list of [`RawRef`] entries on a chunk row. Replaces any previous
/// value.
pub fn set_chunk_raw_refs(config: &Config, chunk_id: &str, refs: &[RawRef]) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_raw_refs(&engine_config(config), chunk_id, refs)
}

/// Stash raw archive pointers on a chunk row inside a caller-owned transaction.
pub fn set_chunk_raw_refs_tx(tx: &Transaction<'_>, chunk_id: &str, refs: &[RawRef]) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_raw_refs_tx(tx, chunk_id, refs)
}

/// Return the raw-archive pointers stored in SQLite for `chunk_id`, or `None`.
pub fn get_chunk_raw_refs(config: &Config, chunk_id: &str) -> Result<Option<Vec<RawRef>>> {
    tinycortex::memory::chunks::get_chunk_raw_refs(&engine_config(config), chunk_id)
}

/// Collect every raw-archive path referenced by any chunk row, restricted to
/// paths under `rel_prefix`.
pub fn list_chunk_raw_ref_paths_with_prefix(
    config: &Config,
    rel_prefix: &str,
) -> Result<std::collections::HashSet<String>> {
    tinycortex::memory::chunks::list_chunk_raw_ref_paths_with_prefix(
        &engine_config(config),
        rel_prefix,
    )
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `chunk_id`.
pub fn get_chunk_content_pointers(
    config: &Config,
    chunk_id: &str,
) -> Result<Option<(String, String)>> {
    tinycortex::memory::chunks::get_chunk_content_pointers(&engine_config(config), chunk_id)
}

/// Return the `content_path` stored in SQLite for `chunk_id`, if any.
pub fn get_chunk_content_path(config: &Config, chunk_id: &str) -> Result<Option<String>> {
    tinycortex::memory::chunks::get_chunk_content_path(&engine_config(config), chunk_id)
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `summary_id`.
pub fn get_summary_content_pointers(
    config: &Config,
    summary_id: &str,
) -> Result<Option<(String, String)>> {
    tinycortex::memory::chunks::get_summary_content_pointers(&engine_config(config), summary_id)
}

/// List all summary rows that have a non-NULL `content_path`.
pub fn list_summaries_with_content_path(config: &Config) -> Result<Vec<(String, String, String)>> {
    tinycortex::memory::chunks::list_summaries_with_content_path(&engine_config(config))
}
