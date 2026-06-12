//! Raw-archive pointers and content-pointer accessors for chunk/summary rows.
//!
//! `RawRef` lets ingest pipelines mirror full message bodies to on-disk
//! archives under `<content_root>/raw/` while storing only a ≤500-char
//! preview in the SQLite `content` column. Retrieval reads the archive
//! directly instead of going through the SQL preview path.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};

use super::with_connection;
use crate::openhuman::config::Config;

/// One pointer into the raw archive. A chunk's body is reconstructed by
/// reading each [`RawRef`] in order and joining with `"\n\n"`.
///
/// `start` / `end` are byte offsets into the raw `.md` file. `end =
/// None` means "read to end of file". Both default to "the whole
/// file" (`start = 0`, `end = None`) for the common one-message-one-chunk
/// path; oversize-message chunks get explicit ranges so each chunk
/// reconstructs its sub-slice.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RawRef {
    /// Forward-slash relative path under `<content_root>/`,
    /// e.g. `"raw/gmail-stevent95-at-gmail-dot-com/1700000_msg-id.md"`.
    pub path: String,
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: Option<usize>,
}

/// Stash a list of [`RawRef`] entries on a chunk row. Replaces any
/// previous value. Used by ingest pipelines that mirror their bytes
/// into `<content_root>/raw/...` so reads can skip the SQL preview
/// path and pull the full body straight from the archive.
pub fn set_chunk_raw_refs(config: &Config, chunk_id: &str, refs: &[RawRef]) -> Result<()> {
    let json = serde_json::to_string(refs).context("serialize raw_refs")?;
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE mem_tree_chunks SET raw_refs_json = ?1 WHERE id = ?2",
            params![json, chunk_id],
        )?;
        Ok(())
    })
}

/// Stash raw archive pointers on a chunk row inside a caller-owned
/// transaction. Used by ingest producers so raw-backed chunks are readable
/// before their `extract_chunk` jobs are committed.
pub fn set_chunk_raw_refs_tx(tx: &Transaction<'_>, chunk_id: &str, refs: &[RawRef]) -> Result<()> {
    let json = serde_json::to_string(refs).context("serialize raw_refs")?;
    tx.execute(
        "UPDATE mem_tree_chunks SET raw_refs_json = ?1 WHERE id = ?2",
        params![json, chunk_id],
    )?;
    Ok(())
}

/// Return the raw-archive pointers stored in SQLite for `chunk_id`,
/// or `None` if no `raw_refs_json` was recorded.
pub fn get_chunk_raw_refs(config: &Config, chunk_id: &str) -> Result<Option<Vec<RawRef>>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT raw_refs_json FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        match row {
            Some(json) if !json.is_empty() => {
                let refs: Vec<RawRef> =
                    serde_json::from_str(&json).context("deserialize raw_refs_json")?;
                Ok(Some(refs))
            }
            _ => Ok(None),
        }
    })
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `chunk_id`.
///
/// Returns `Ok(None)` if the chunk does not exist or has no content_path recorded yet.
pub fn get_chunk_content_pointers(
    config: &Config,
    chunk_id: &str,
) -> Result<Option<(String, String)>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path, content_sha256 FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| {
                    let path: Option<String> = r.get(0)?;
                    let sha: Option<String> = r.get(1)?;
                    Ok((path, sha))
                },
            )
            .optional()?;
        Ok(row.and_then(|(p, s)| p.zip(s)))
    })
}

/// Return the `content_path` stored in SQLite for `chunk_id`, if any.
pub fn get_chunk_content_path(config: &Config, chunk_id: &str) -> Result<Option<String>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(row)
    })
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `summary_id`.
///
/// Returns `Ok(None)` if the summary does not exist or has no content_path recorded yet
/// (legacy rows pre-MD-content migration).
pub fn get_summary_content_pointers(
    config: &Config,
    summary_id: &str,
) -> Result<Option<(String, String)>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path, content_sha256 FROM mem_tree_summaries WHERE id = ?1",
                params![summary_id],
                |r| {
                    let path: Option<String> = r.get(0)?;
                    let sha: Option<String> = r.get(1)?;
                    Ok((path, sha))
                },
            )
            .optional()?;
        Ok(row.and_then(|(p, s)| p.zip(s)))
    })
}

/// List all summary rows that have a non-NULL `content_path`. Used by the
/// bin integrity checker.
pub fn list_summaries_with_content_path(config: &Config) -> Result<Vec<(String, String, String)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, content_path, content_sha256
               FROM mem_tree_summaries
              WHERE content_path IS NOT NULL AND content_sha256 IS NOT NULL
                AND deleted = 0",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id: String = r.get(0)?;
                let path: String = r.get(1)?;
                let sha: String = r.get(2)?;
                Ok((id, path, sha))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to list summaries with content_path")?;
        Ok(rows)
    })
}
