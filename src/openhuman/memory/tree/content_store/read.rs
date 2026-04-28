//! Read and verify chunk and summary `.md` files from the content store.

use std::path::Path;

use super::atomic::sha256_hex;
use super::compose::split_front_matter;

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
        log::warn!(
            "[content_store::read] sha256 mismatch for {}: expected={} actual={}",
            abs_path.display(),
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
        log::warn!(
            "[content_store::read] sha256 mismatch for summary {}: expected={} actual={}",
            abs_path.display(),
            expected_sha256,
            contents.sha256,
        );
        Ok(VerifyResult::Mismatch {
            actual: contents.sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::content_store::atomic::{sha256_hex, write_if_new};
    use crate::openhuman::memory::tree::content_store::compose::compose_chunk_file;
    use crate::openhuman::memory::tree::types::{Chunk, Metadata, SourceKind};
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
        use crate::openhuman::memory::tree::content_store::atomic::{sha256_hex, write_if_new};
        use crate::openhuman::memory::tree::content_store::compose::{
            compose_summary_md, SummaryComposeInput,
        };
        use crate::openhuman::memory::tree::content_store::paths::SummaryTreeKind;
        use chrono::TimeZone;
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "sum:L1:readtest",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &["c1".to_string()],
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
}
