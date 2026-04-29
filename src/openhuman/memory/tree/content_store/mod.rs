//! Content store for memory-tree chunk and summary `.md` files (Phase MD-content).
//!
//! Bodies are stored on disk as `.md` files with YAML front-matter.
//! SQLite holds `content_path` (relative, forward-slash) and `content_sha256`
//! (over body bytes only) as pointers + integrity tokens.
//!
//! ## Module layout
//!
//! - [`paths`]   — path generation + `slugify_source_id` + summary path builders
//! - [`compose`] — YAML front-matter + body composition; tag rewriting
//! - [`atomic`]  — tempfile+fsync+rename writes; SHA-256; `stage_summary`
//! - [`read`]    — read + SHA-256 verification + `split_front_matter`; summary variants
//! - [`tags`]    — `update_chunk_tags` + `update_summary_tags` + slugifiers

pub mod atomic;
pub mod compose;
pub mod paths;
pub mod read;
pub mod tags;

use std::path::Path;

use crate::openhuman::memory::tree::types::Chunk;

pub use atomic::StagedSummary;
pub use compose::SummaryComposeInput;
pub use paths::SummaryTreeKind;

/// A chunk that has been written to disk and is ready for SQLite upsert.
///
/// Callers build a `Vec<StagedChunk>` from `stage_chunks`, then pass it to
/// `store::upsert_chunks_tx` in the same SQLite transaction.
#[derive(Debug, Clone)]
pub struct StagedChunk {
    /// The original chunk (metadata + content).
    pub chunk: Chunk,
    /// Relative content path (forward-slash, e.g. `"chat/slack-eng/0.md"`).
    pub content_path: String,
    /// SHA-256 hex digest over the body bytes only.
    pub content_sha256: String,
}

/// Update the `tags:` block in a summary's on-disk `.md` file after an
/// extraction job runs.
///
/// Delegates to [`tags::update_summary_tags`].
pub fn update_summary_tags(
    config: &crate::openhuman::config::Config,
    summary_id: &str,
) -> anyhow::Result<()> {
    tags::update_summary_tags(config, summary_id)
}

/// Write all chunks in `chunks` to disk and return `StagedChunk` records
/// ready for SQLite upsert.
///
/// Each chunk file is written atomically via a sibling temp-file + rename.
/// Already-existing files are skipped (immutable-body contract). Parent
/// directories are created on demand.
///
/// `content_root` — absolute path to the root of the content store.
pub fn stage_chunks(content_root: &Path, chunks: &[Chunk]) -> anyhow::Result<Vec<StagedChunk>> {
    let mut staged = Vec::with_capacity(chunks.len());

    for chunk in chunks {
        let source_kind = chunk.metadata.source_kind.as_str();
        let source_id = &chunk.metadata.source_id;

        let rel_path = paths::chunk_rel_path(source_kind, source_id, &chunk.id);
        let abs_path = paths::chunk_abs_path(content_root, source_kind, source_id, &chunk.id);

        let (full_bytes, body_bytes) = compose::compose_chunk_file(chunk);
        let sha256 = atomic::sha256_hex(&body_bytes);

        match atomic::write_if_new(&abs_path, &full_bytes) {
            Ok(written) => {
                if written {
                    log::debug!("[content_store] wrote chunk {} → {}", chunk.id, rel_path);
                } else {
                    log::debug!(
                        "[content_store] chunk {} already on disk at {}",
                        chunk.id,
                        rel_path
                    );
                }
            }
            Err(e) => {
                log::error!(
                    "[content_store] failed to write chunk {} to {}: {e}",
                    chunk.id,
                    rel_path
                );
                return Err(e);
            }
        }

        staged.push(StagedChunk {
            chunk: chunk.clone(),
            content_path: rel_path,
            content_sha256: sha256,
        });
    }

    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::{Metadata, SourceKind};
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn sample_chunk(seq: u32) -> Chunk {
        let ts = chrono::Utc
            .timestamp_millis_opt(1_700_000_000_000 + seq as i64)
            .unwrap();
        Chunk {
            id: format!("chunk_{seq}"),
            content: format!("## ts — alice\nMessage {seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
            },
            token_count: 5,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn stage_chunks_writes_files_and_returns_staged() {
        let dir = TempDir::new().unwrap();
        let chunks = vec![sample_chunk(0), sample_chunk(1)];
        let staged = stage_chunks(dir.path(), &chunks).unwrap();

        assert_eq!(staged.len(), 2);
        for s in &staged {
            let abs = paths::chunk_abs_path(
                dir.path(),
                s.chunk.metadata.source_kind.as_str(),
                &s.chunk.metadata.source_id,
                &s.chunk.id,
            );
            assert!(abs.exists(), "file must exist: {}", abs.display());
            assert!(!s.content_path.is_empty());
            assert_eq!(s.content_sha256.len(), 64);
            // Path must be relative with forward slashes.
            assert!(!s.content_path.starts_with('/'));
            assert!(s.content_path.contains('/'));
        }
    }

    #[test]
    fn stage_chunks_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let chunks = vec![sample_chunk(0)];
        let first = stage_chunks(dir.path(), &chunks).unwrap();
        let second = stage_chunks(dir.path(), &chunks).unwrap();
        assert_eq!(first[0].content_sha256, second[0].content_sha256);
        assert_eq!(first[0].content_path, second[0].content_path);
    }
}
