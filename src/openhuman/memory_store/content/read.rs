//! Read and verify chunk and summary `.md` files from the content store.

use std::path::Path;

use super::atomic::sha256_hex;
use super::compose::split_front_matter;
use crate::openhuman::memory::util::redact::redact;

/// The result of reading a chunk file from disk.
pub struct ChunkFileContents {
    /// The Markdown body (everything after the closing `---` of the front-matter).
    pub body: String,
    /// SHA-256 hex digest over the **body bytes** only.
    pub sha256: String,
}

/// Read a chunk file and return its body + SHA-256.
///
/// Returns an error if:
/// - the file does not exist
/// - the file is not valid UTF-8
/// - the front-matter delimiters cannot be found
pub fn read_chunk_file(abs_path: &Path) -> anyhow::Result<ChunkFileContents> {
    let raw = std::fs::read(abs_path).map_err(|e| anyhow::anyhow!("read {:?}: {e}", abs_path))?;
    let content = std::str::from_utf8(&raw)
        .map_err(|e| anyhow::anyhow!("invalid UTF-8 in {:?}: {e}", abs_path))?;

    let (_fm, body) = split_front_matter(content)
        .ok_or_else(|| anyhow::anyhow!("no front-matter in {:?}", abs_path))?;

    let sha256 = sha256_hex(body.as_bytes());
    Ok(ChunkFileContents {
        body: body.to_string(),
        sha256,
    })
}

/// Verify that the body of a chunk file matches the expected SHA-256.
///
/// Returns `Ok(true)` on a match, `Ok(false)` on a mismatch, and an `Err`
/// if the file cannot be read or parsed.
pub fn verify_chunk_file(abs_path: &Path, expected_sha256: &str) -> anyhow::Result<bool> {
    let contents = read_chunk_file(abs_path)?;
    let ok = contents.sha256 == expected_sha256;
    if !ok {
        // Log the path as a redacted hash — the path may embed email addresses
        // (participant slugs) after the participant-bucketing change.
        let path_str = abs_path.to_string_lossy();
        log::warn!(
            "[content_store::read] sha256 mismatch for path_hash={}: expected={} actual={}",
            redact(&path_str),
            expected_sha256,
            contents.sha256,
        );
    }
    Ok(ok)
}

// ── Summary reads ────────────────────────────────────────────────────────────

/// The result of verifying a summary file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyResult {
    /// The on-disk body SHA-256 matches the stored value.
    Ok,
    /// The file exists but the body SHA-256 does not match.
    Mismatch { actual: String },
    /// The file does not exist at the given path.
    Missing,
}

/// Read a summary file and return its body + SHA-256.
///
/// Returns an error if:
/// - the file does not exist
/// - the file is not valid UTF-8
/// - the front-matter delimiters cannot be found
pub fn read_summary_file(abs_path: &Path) -> anyhow::Result<ChunkFileContents> {
    // Reuse the same reader as chunks — the file format is identical.
    read_chunk_file(abs_path)
}

/// Verify a summary file's body SHA-256 without returning the body itself.
///
/// Returns:
/// - `VerifyResult::Ok` on match
/// - `VerifyResult::Mismatch { actual }` on hash mismatch
/// - `VerifyResult::Missing` when the file does not exist
pub fn verify_summary_file(abs_path: &Path, expected_sha256: &str) -> anyhow::Result<VerifyResult> {
    if !abs_path.exists() {
        return Ok(VerifyResult::Missing);
    }
    let contents = read_summary_file(abs_path)?;
    if contents.sha256 == expected_sha256 {
        Ok(VerifyResult::Ok)
    } else {
        // Redact the path — it can embed participant slugs (email addresses).
        let path_str = abs_path.to_string_lossy();
        log::warn!(
            "[content_store::read] sha256 mismatch for summary path_hash={}: expected={} actual={}",
            redact(&path_str),
            expected_sha256,
            contents.sha256,
        );
        Ok(VerifyResult::Mismatch {
            actual: contents.sha256,
        })
    }
}

// ── High-level body readers (Config-aware) ───────────────────────────────────
//
// These helpers resolve the on-disk path from SQLite via
// `get_chunk_content_pointers` / `get_summary_content_pointers`, then read the
// file body. They are the single authoritative entry-point for every caller
// that needs the **full** chunk or summary body (LLM extractor, summariser
// inputs, retrieval API, embedder). Preview-only consumers (UI cards, fast
// filter scans) continue reading the `content` column directly from SQLite.
//
// Error policy:
// - If `content_path` / `content_sha256` are NULL (legacy rows ingested before
//   the MD-on-disk migration), return `Err` — callers must handle the
//   "pre-migration chunk" case explicitly. The job pipeline propagates the
//   error and retries; retrieval falls back gracefully.
// - File-not-found or SHA mismatch → `Err` (propagated to caller for retry /
//   alerting).

/// Read the full body of a chunk `.md` file by its chunk id.
///
/// Looks up `content_path` in SQLite, resolves it to an absolute path under
/// `config.memory_tree_content_root()`, reads the file, and returns the body
/// string (everything after the YAML front-matter delimiter).
///
/// Returns `Err` if:
/// - The chunk row has no `content_path` recorded (pre-MD-migration row).
/// - The file cannot be read or has no valid front-matter.
///
/// # Preview vs. full body
/// The `content` column in `mem_tree_chunks` holds a ≤500-char preview after
/// the MD-on-disk migration. Use this function wherever the full body is
/// required (LLM extraction, embedding, summariser inputs, retrieval API).
pub fn read_chunk_body(
    config: &crate::openhuman::config::Config,
    chunk_id: &str,
) -> anyhow::Result<String> {
    use crate::openhuman::memory_store::chunks::store::{
        get_chunk_content_pointers, get_chunk_raw_refs,
    };

    // Path 1: chunk has raw-archive pointers (today: email). Read each
    // referenced file, slice by byte range, join with `\n\n` (the
    // chunker's unit separator). No SHA verify — the raw archive is
    // the source of truth and was written transactionally with the
    // chunk row's id; mismatch can only happen after manual edits.
    if let Some(refs) = get_chunk_raw_refs(config, chunk_id)? {
        if !refs.is_empty() {
            return read_chunk_body_from_raw(config, &refs);
        }
    }

    let pointers = get_chunk_content_pointers(config, chunk_id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "[content_store::read] no content_path or raw_refs for chunk_id={} \
             (pre-MD-migration row?)",
            chunk_id
        )
    })?;
    let (rel_path, expected_sha256) = pointers;
    if rel_path.is_empty() {
        return Err(anyhow::anyhow!(
            "[content_store::read] empty content_path and no raw_refs for chunk_id={} \
             — chunk has no resolvable body source",
            chunk_id
        ));
    }

    let content_root = config.memory_tree_content_root();
    // Reconstruct the absolute path from the stored relative forward-slash path.
    let abs_path = {
        let mut p = content_root.clone();
        for component in rel_path.split('/') {
            p.push(component);
        }
        p
    };

    log::debug!(
        "[content_store::read] read_chunk_body chunk_id={} path_hash={}",
        chunk_id,
        redact(&rel_path),
    );

    let result = read_chunk_file(&abs_path).with_context(|| {
        format!(
            "read_chunk_body: failed to read file for chunk_id={} path_hash={}",
            chunk_id,
            redact(&rel_path),
        )
    })?;

    // Verify the on-disk body matches the SHA stored at write time. A mismatch
    // means the file was tampered with, the tx that committed the pointer
    // raced with a separate writer, or the disk corrupted — all unsafe to
    // hand back to a consumer. Fail loudly rather than serve stale/corrupt
    // bytes into the LLM extractor / summariser pipeline.
    if result.sha256 != expected_sha256 {
        return Err(anyhow::anyhow!(
            "[content_store::read] sha256 mismatch for chunk_id={} \
             expected={} actual={} path_hash={}",
            chunk_id,
            expected_sha256,
            result.sha256,
            redact(&rel_path),
        ));
    }

    Ok(result.body)
}

use anyhow::Context as _;

/// Reconstruct a chunk body by reading the raw archive files it
/// points at and joining their contents with `"\n\n"` — the same
/// separator the chunker uses between units.
///
/// Each [`RawRef`] is resolved relative to
/// `config.memory_tree_content_root()`. Byte ranges (`start`, `end`)
/// slice the file; defaults read the whole file. Out-of-bounds
/// ranges are clamped (start past EOF returns empty, end past EOF
/// reads to EOF) so a corrupted offset can't panic the worker —
/// reads are best-effort, log + skip on per-file errors so a single
/// missing raw file doesn't take the whole chunk down.
fn read_chunk_body_from_raw(
    config: &crate::openhuman::config::Config,
    refs: &[crate::openhuman::memory_store::chunks::store::RawRef],
) -> anyhow::Result<String> {
    let content_root = config.memory_tree_content_root();
    let mut parts: Vec<String> = Vec::with_capacity(refs.len());
    for r in refs {
        let mut abs = content_root.clone();
        for component in r.path.split('/') {
            abs.push(component);
        }
        let bytes = match std::fs::read(&abs) {
            Ok(b) => b,
            Err(e) => {
                log::warn!(
                    "[content_store::read] raw_ref read failed path_hash={} err={e}",
                    redact(&r.path)
                );
                continue;
            }
        };
        let len = bytes.len();
        let start = r.start.min(len);
        let end = r.end.unwrap_or(len).min(len);
        if end <= start {
            continue;
        }
        let slice = &bytes[start..end];
        match std::str::from_utf8(slice) {
            Ok(s) => parts.push(s.to_string()),
            Err(e) => {
                log::warn!(
                    "[content_store::read] raw_ref non-utf8 path_hash={} err={e}",
                    redact(&r.path)
                );
            }
        }
    }
    Ok(parts.join("\n\n"))
}

/// Read the full body of a summary `.md` file by its summary id.
///
/// Looks up `content_path` in SQLite, resolves it to an absolute path under
/// `config.memory_tree_content_root()`, reads the file, and returns the body
/// string.
///
/// Returns `Err` if:
/// - The summary row has no `content_path` recorded (pre-MD-migration row).
/// - The file cannot be read or has no valid front-matter.
///
/// # Preview vs. full body
/// The `content` column in `mem_tree_summaries` holds a ≤500-char preview after
/// the MD-on-disk migration. Use this function wherever the full body is
/// required (LLM extraction, embedding, summariser inputs, retrieval API).
pub fn read_summary_body(
    config: &crate::openhuman::config::Config,
    summary_id: &str,
) -> anyhow::Result<String> {
    use crate::openhuman::memory_store::chunks::store::get_summary_content_pointers;

    let pointers = get_summary_content_pointers(config, summary_id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "[content_store::read] no content_path for summary_id={} (pre-MD-migration row?)",
            summary_id
        )
    })?;
    let (rel_path, expected_sha256) = pointers;

    let content_root = config.memory_tree_content_root();
    let abs_path = {
        let mut p = content_root.clone();
        for component in rel_path.split('/') {
            p.push(component);
        }
        p
    };

    log::debug!(
        "[content_store::read] read_summary_body summary_id={} path_hash={}",
        summary_id,
        redact(&rel_path),
    );

    let result = read_summary_file(&abs_path).with_context(|| {
        format!(
            "read_summary_body: failed to read file for summary_id={} path_hash={}",
            summary_id,
            redact(&rel_path),
        )
    })?;

    // Verify the on-disk body matches the SHA stored at seal time. See the
    // matching guard in `read_chunk_body` for rationale.
    if result.sha256 != expected_sha256 {
        return Err(anyhow::anyhow!(
            "[content_store::read] sha256 mismatch for summary_id={} \
             expected={} actual={} path_hash={}",
            summary_id,
            expected_sha256,
            result.sha256,
            redact(&rel_path),
        ));
    }

    Ok(result.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::memory_store::chunks::store::{upsert_chunks, with_connection};
    use crate::openhuman::memory_store::chunks::types::{Chunk, Metadata, SourceKind};
    use crate::openhuman::memory_store::content::atomic::{sha256_hex, write_if_new};
    use crate::openhuman::memory_store::content::compose::{
        compose_chunk_file, SummaryComposeInput,
    };
    use crate::openhuman::memory_store::content::paths::SummaryTreeKind;
    use crate::openhuman::memory_store::content::{atomic::stage_summary, stage_chunks};
    use crate::openhuman::memory_store::trees::store::{insert_summary_tx, insert_tree};
    use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeKind, TreeStatus};
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn sample_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "read_test".into(),
            content: "## ts — alice\nhello from read test".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
            },
            token_count: 8,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg
    }

    fn sample_tree() -> Tree {
        Tree {
            id: "tree-1".into(),
            kind: TreeKind::Source,
            scope: "slack:#eng".into(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            last_sealed_at: None,
        }
    }

    fn sample_summary_node() -> SummaryNode {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        SummaryNode {
            id: "summary-1".into(),
            tree_id: "tree-1".into(),
            tree_kind: TreeKind::Source,
            level: 1,
            parent_id: None,
            child_ids: vec!["leaf-a".into()],
            content: "summary full body".into(),
            token_count: 4,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
        }
    }

    #[test]
    fn read_returns_body_and_correct_sha256() {
        let dir = TempDir::new().unwrap();
        let chunk = sample_chunk();
        let (full_bytes, body_bytes) = compose_chunk_file(&chunk);
        let path = dir.path().join("0.md");
        write_if_new(&path, &full_bytes).unwrap();

        let result = read_chunk_file(&path).unwrap();
        assert_eq!(result.body, std::str::from_utf8(&body_bytes).unwrap());
        assert_eq!(result.sha256, sha256_hex(&body_bytes));
    }

    #[test]
    fn verify_passes_for_correct_hash() {
        let dir = TempDir::new().unwrap();
        let chunk = sample_chunk();
        let (full_bytes, body_bytes) = compose_chunk_file(&chunk);
        let path = dir.path().join("0.md");
        write_if_new(&path, &full_bytes).unwrap();

        let expected = sha256_hex(&body_bytes);
        assert!(verify_chunk_file(&path, &expected).unwrap());
    }

    #[test]
    fn verify_fails_for_wrong_hash() {
        let dir = TempDir::new().unwrap();
        let chunk = sample_chunk();
        let (full_bytes, _) = compose_chunk_file(&chunk);
        let path = dir.path().join("0.md");
        write_if_new(&path, &full_bytes).unwrap();

        assert!(!verify_chunk_file(&path, "deadbeef").unwrap());
    }

    #[test]
    fn read_missing_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        assert!(read_chunk_file(&path).is_err());
    }

    // ─── summary read / verify tests ─────────────────────────────────────────

    fn write_summary_file(dir: &TempDir, body: &str) -> (std::path::PathBuf, String) {
        use crate::openhuman::memory_store::content::atomic::{sha256_hex, write_if_new};
        use crate::openhuman::memory_store::content::compose::{
            compose_summary_md, SummaryComposeInput,
        };
        use crate::openhuman::memory_store::content::paths::SummaryTreeKind;
        use chrono::TimeZone;
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "sum:L1:readtest",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &["c1".to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body,
        };
        let composed = compose_summary_md(&input);
        let path = dir.path().join("sum.md");
        let sha = sha256_hex(composed.body.as_bytes());
        write_if_new(&path, composed.full.as_bytes()).unwrap();
        (path, sha)
    }

    #[test]
    fn read_summary_file_returns_body_and_sha() {
        let dir = TempDir::new().unwrap();
        let body = "summary body content\n";
        let (path, expected_sha) = write_summary_file(&dir, body);
        let result = read_summary_file(&path).unwrap();
        assert_eq!(result.body, body);
        assert_eq!(result.sha256, expected_sha);
    }

    #[test]
    fn verify_summary_file_ok_for_correct_hash() {
        let dir = TempDir::new().unwrap();
        let (path, sha) = write_summary_file(&dir, "body text\n");
        assert_eq!(verify_summary_file(&path, &sha).unwrap(), VerifyResult::Ok);
    }

    #[test]
    fn verify_summary_file_mismatch_for_wrong_hash() {
        let dir = TempDir::new().unwrap();
        let (path, _) = write_summary_file(&dir, "body text\n");
        let r = verify_summary_file(&path, "deadbeef").unwrap();
        assert!(matches!(r, VerifyResult::Mismatch { .. }));
    }

    #[test]
    fn verify_summary_file_missing_for_absent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does_not_exist.md");
        assert_eq!(
            verify_summary_file(&path, "abc").unwrap(),
            VerifyResult::Missing
        );
    }

    #[test]
    fn read_chunk_file_rejects_invalid_utf8() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.md");
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        let err = match read_chunk_file(&path) {
            Ok(_) => panic!("invalid UTF-8 should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid UTF-8"));
    }

    #[test]
    fn read_chunk_file_rejects_missing_front_matter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("plain.md");
        std::fs::write(&path, "no front matter here").unwrap();
        let err = match read_chunk_file(&path) {
            Ok(_) => panic!("missing front matter should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("no front-matter"));
    }

    #[test]
    fn verify_summary_file_mismatch_returns_actual_sha() {
        let dir = TempDir::new().unwrap();
        let (path, expected_sha) = write_summary_file(&dir, "body text\n");
        let actual = match verify_summary_file(&path, "deadbeef").unwrap() {
            VerifyResult::Mismatch { actual } => actual,
            other => panic!("expected mismatch, got {other:?}"),
        };
        assert_eq!(actual, expected_sha);
    }

    #[test]
    fn read_chunk_body_from_raw_clamps_ranges_and_skips_bad_refs() {
        use crate::openhuman::memory_store::chunks::store::RawRef;

        let dir = TempDir::new().unwrap();
        let mut cfg = crate::openhuman::config::Config::default();
        cfg.workspace_dir = dir.path().to_path_buf();

        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();

        std::fs::write(content_root.join("one.txt"), "abcdef").unwrap();
        std::fs::write(content_root.join("two.txt"), [0xff, 0xfe]).unwrap();

        let refs = vec![
            RawRef {
                path: "one.txt".into(),
                start: 1,
                end: Some(4),
            },
            RawRef {
                path: "missing.txt".into(),
                start: 0,
                end: None,
            },
            RawRef {
                path: "two.txt".into(),
                start: 0,
                end: None,
            },
            RawRef {
                path: "one.txt".into(),
                start: 99,
                end: None,
            },
        ];

        let body = read_chunk_body_from_raw(&cfg, &refs).unwrap();
        assert_eq!(body, "bcd");
    }

    #[test]
    fn read_chunk_body_roundtrips_from_staged_content_pointer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let chunk = sample_chunk();
        upsert_chunks(&cfg, std::slice::from_ref(&chunk)).unwrap();
        let staged = stage_chunks(
            &cfg.memory_tree_content_root(),
            std::slice::from_ref(&chunk),
        )
        .unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let body = read_chunk_body(&cfg, &chunk.id).unwrap();
        assert_eq!(body, chunk.content);
    }

    #[test]
    fn read_chunk_body_errors_when_pointers_are_missing() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let err = read_chunk_body(&cfg, "missing-chunk").unwrap_err();
        assert!(err.to_string().contains("no content_path or raw_refs"));
    }

    #[test]
    fn read_chunk_body_errors_on_sha_mismatch() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let chunk = sample_chunk();
        upsert_chunks(&cfg, std::slice::from_ref(&chunk)).unwrap();
        let staged = stage_chunks(
            &cfg.memory_tree_content_root(),
            std::slice::from_ref(&chunk),
        )
        .unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let rel =
            crate::openhuman::memory_store::chunks::store::get_chunk_content_path(&cfg, &chunk.id)
                .unwrap()
                .unwrap();
        let mut abs = cfg.memory_tree_content_root();
        for part in rel.split('/') {
            abs.push(part);
        }
        std::fs::write(&abs, b"---\nsource_kind: chat\n---\nmutated body").unwrap();

        let err = read_chunk_body(&cfg, &chunk.id).unwrap_err();
        assert!(err.to_string().contains("sha256 mismatch"));
    }

    #[test]
    fn read_summary_body_roundtrips_from_staged_content_pointer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let tree = sample_tree();
        let node = sample_summary_node();
        insert_tree(&cfg, &tree).unwrap();
        let staged = stage_summary(
            &cfg.memory_tree_content_root(),
            &SummaryComposeInput {
                summary_id: &node.id,
                tree_kind: SummaryTreeKind::Source,
                tree_id: &tree.id,
                tree_scope: &tree.scope,
                level: node.level,
                child_ids: &node.child_ids,
                child_basenames: None,
                child_count: node.child_ids.len(),
                time_range_start: node.time_range_start,
                time_range_end: node.time_range_end,
                sealed_at: node.sealed_at,
                body: &node.content,
            },
            "slack-eng",
            None,
        )
        .unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            insert_summary_tx(&tx, &node, Some(&staged), "test")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let body = read_summary_body(&cfg, &node.id).unwrap();
        assert_eq!(body, node.content);
    }

    #[test]
    fn read_summary_body_errors_when_pointers_are_missing() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let err = read_summary_body(&cfg, "missing-summary").unwrap_err();
        assert!(err.to_string().contains("no content_path for summary_id"));
    }
}
