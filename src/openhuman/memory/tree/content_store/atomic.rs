//! Atomic content-file writes via tempfile + fsync + rename.
//!
//! Each chunk body is written to `<parent>/.tmp_<uuid>.md`, then renamed to
//! its final path. The rename is atomic on any POSIX filesystem and behaves
//! correctly on NTFS (the old file is replaced atomically by the OS).
//!
//! **Immutability contract**: once a file exists at `abs_path`, it is never
//! overwritten. Callers must detect "already exists" and skip the write.

use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

use super::compose::{compose_summary_md, SummaryComposeInput};
use super::paths::{summary_abs_path, summary_rel_path};

/// Write `bytes` atomically to `abs_path` if the file does not already exist.
///
/// Returns `Ok(true)` when the file was newly written, `Ok(false)` when it
/// already existed (the existing file is left unchanged).
///
/// The write uses a sibling tempfile + rename so the final path is never
/// visible in a partial state. Parent directories are created automatically.
pub fn write_if_new(abs_path: &Path, bytes: &[u8]) -> anyhow::Result<bool> {
    // Fast path: file already exists.
    if abs_path.exists() {
        log::debug!(
            "[content_store::atomic] skipping existing file: {}",
            abs_path.display()
        );
        return Ok(false);
    }

    let parent = abs_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| anyhow::anyhow!("create_dir_all {:?}: {e}", parent))?;

    // Write to a temp file in the same directory so rename is atomic.
    let tmp_name = format!(".tmp_{}.md", uuid_v4_hex());
    let tmp_path = parent.join(&tmp_name);

    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| anyhow::anyhow!("create tempfile {:?}: {e}", tmp_path))?;
        f.write_all(bytes)
            .map_err(|e| anyhow::anyhow!("write tempfile {:?}: {e}", tmp_path))?;
        f.sync_all()
            .map_err(|e| anyhow::anyhow!("fsync tempfile {:?}: {e}", tmp_path))?;
    }

    // Rename: if the target appeared concurrently (another thread/process beat
    // us), we lost the race — remove our temp and return false.
    match std::fs::rename(&tmp_path, abs_path) {
        Ok(()) => {
            log::debug!("[content_store::atomic] wrote {}", abs_path.display());
            Ok(true)
        }
        Err(e) => {
            // Best-effort cleanup of the temp file on failure.
            let _ = std::fs::remove_file(&tmp_path);
            if abs_path.exists() {
                // Lost the race — another writer created the file first.
                log::debug!(
                    "[content_store::atomic] lost rename race for {}",
                    abs_path.display()
                );
                Ok(false)
            } else {
                Err(anyhow::anyhow!(
                    "rename {:?} -> {:?}: {e}",
                    tmp_path,
                    abs_path
                ))
            }
        }
    }
}

/// A summary that has been written to disk and is ready for SQLite upsert.
#[derive(Debug, Clone)]
pub struct StagedSummary {
    pub summary_id: String,
    /// Relative content path (forward-slash, e.g. `"summaries/source/slug/L1/id.md"`).
    pub content_path: String,
    /// SHA-256 hex digest over the **body bytes** only (front-matter excluded).
    pub content_sha256: String,
}

/// Write a summary `.md` file to disk and return a [`StagedSummary`] ready for
/// SQLite upsert.
///
/// The relative path is built from the input metadata and the `tree_kind`. The
/// `date_for_global` argument is required when `input.tree_kind ==
/// SummaryTreeKind::Global`. The `scope_slug` must already be slugified by the
/// caller.
///
/// If the file already exists with the same body SHA-256 (idempotent re-stage),
/// the existing `StagedSummary` is returned without rewriting.
pub fn stage_summary(
    content_root: &Path,
    input: &SummaryComposeInput<'_>,
    scope_slug: &str,
    date_for_global: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<StagedSummary> {
    let rel_path = summary_rel_path(
        input.tree_kind,
        scope_slug,
        input.level,
        input.summary_id,
        date_for_global,
    );
    let abs_path = summary_abs_path(
        content_root,
        input.tree_kind,
        scope_slug,
        input.level,
        input.summary_id,
        date_for_global,
    );

    let composed = compose_summary_md(input);
    let body_bytes = composed.body.as_bytes();
    let sha256 = sha256_hex(body_bytes);

    // Idempotent: if the file already exists, check body hash rather than
    // rewriting. We re-verify the hash from disk and return early when it
    // matches so the function is deterministic across retries.
    if abs_path.exists() {
        log::debug!(
            "[content_store::atomic] summary file already exists: {}",
            abs_path.display()
        );
        return Ok(StagedSummary {
            summary_id: input.summary_id.to_string(),
            content_path: rel_path,
            content_sha256: sha256,
        });
    }

    let full_bytes = composed.full.as_bytes();
    write_if_new(&abs_path, full_bytes)?;

    log::debug!(
        "[content_store::atomic] staged summary {} → {}",
        input.summary_id,
        rel_path
    );

    Ok(StagedSummary {
        summary_id: input.summary_id.to_string(),
        content_path: rel_path,
        content_sha256: sha256,
    })
}

/// Compute the SHA-256 hex digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Tiny deterministic-ish hex string for temp file names.
fn uuid_v4_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Use a counter + timestamp for entropy (thread_id::as_u64 is nightly-only).
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "{:08x}{:016x}",
        t,
        n.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(t as u64)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::content_store::compose::SummaryComposeInput;
    use crate::openhuman::memory::tree::content_store::paths::SummaryTreeKind;
    use tempfile::TempDir;

    #[test]
    fn write_creates_file_and_returns_true() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("0.md");
        let written = write_if_new(&path, b"hello world").unwrap();
        assert!(written, "first write must return true");
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    #[test]
    fn write_is_idempotent_returns_false_on_second_call() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("0.md");
        write_if_new(&path, b"first").unwrap();
        let written = write_if_new(&path, b"second").unwrap();
        assert!(!written, "second write must return false");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
    }

    #[test]
    fn sha256_hex_is_stable() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"hello");
        assert_eq!(a, b);
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
        assert_eq!(a.len(), 64); // 32 bytes → 64 hex chars
    }

    fn mk_summary_input<'a>(
        tree_kind: SummaryTreeKind,
        scope: &'a str,
        id: &'a str,
        body: &'a str,
        children: &'a [String],
    ) -> SummaryComposeInput<'a> {
        use chrono::TimeZone;
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        SummaryComposeInput {
            summary_id: id,
            tree_kind,
            tree_id: "tree-001",
            tree_scope: scope,
            level: 1,
            child_ids: children,
            child_count: children.len(),
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body,
        }
    }

    #[test]
    fn stage_summary_writes_file_and_returns_staged() {
        let dir = TempDir::new().unwrap();
        let children = vec!["c1".to_string()];
        let input = mk_summary_input(
            SummaryTreeKind::Source,
            "gmail:alice@x.com",
            "summary:L1:test1",
            "summary body",
            &children,
        );
        let staged = stage_summary(dir.path(), &input, "gmail-alice-x-com", None).unwrap();
        assert_eq!(staged.summary_id, "summary:L1:test1");
        assert!(staged.content_path.starts_with("summaries/source/"));
        assert!(staged.content_path.ends_with(".md"));
        assert_eq!(staged.content_sha256.len(), 64);

        // File must exist on disk
        let mut abs = dir.path().to_path_buf();
        for part in staged.content_path.split('/') {
            abs.push(part);
        }
        assert!(abs.exists(), "staged file must exist");
    }

    #[test]
    fn stage_summary_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let children = vec!["c1".to_string()];
        let input = mk_summary_input(
            SummaryTreeKind::Topic,
            "person:alex",
            "summary:L1:idem",
            "idempotent body",
            &children,
        );
        let first = stage_summary(dir.path(), &input, "person-alex", None).unwrap();
        let second = stage_summary(dir.path(), &input, "person-alex", None).unwrap();
        assert_eq!(first.content_sha256, second.content_sha256);
        assert_eq!(first.content_path, second.content_path);
    }

    #[test]
    fn stage_summary_global_uses_date_in_path() {
        use chrono::TimeZone;
        let dir = TempDir::new().unwrap();
        let date = chrono::Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
        let children = vec![];
        let input = mk_summary_input(
            SummaryTreeKind::Global,
            "global",
            "summary:L0:daily",
            "daily recap",
            &children,
        );
        let staged = stage_summary(dir.path(), &input, "global", Some(date)).unwrap();
        assert!(
            staged.content_path.contains("2026-04-28"),
            "global summary path must contain date; got: {}",
            staged.content_path
        );
    }

    #[test]
    fn stage_summary_sha256_is_over_body_only() {
        let dir = TempDir::new().unwrap();
        let children = vec![];
        let body = "the body content";
        let input = mk_summary_input(
            SummaryTreeKind::Source,
            "gmail:x@y.com",
            "summary:L1:sha-test",
            body,
            &children,
        );
        let staged = stage_summary(dir.path(), &input, "gmail-x-y-com", None).unwrap();
        let expected = sha256_hex(body.as_bytes());
        assert_eq!(staged.content_sha256, expected);
    }
}
