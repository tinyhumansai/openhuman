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

use super::compose::{compose_summary_md, split_front_matter, SummaryComposeInput};
use super::paths::{summary_rel_path_with_layout, SummaryDiskLayout};

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
        // Remove the temp file on write/fsync failure so a leaked
        // `.tmp_<uuid>.md` never accumulates in the user-facing vault.
        if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("write/fsync tempfile {:?}: {e}", tmp_path));
        }
    }

    // Rename: if the target appeared concurrently (another thread/process beat
    // us), we lost the race — remove our temp and return false.
    match std::fs::rename(&tmp_path, abs_path) {
        Ok(()) => {
            // fsync the parent directory so the rename (directory entry
            // update) is durable across a crash or power loss. Without this,
            // sync_all() on the file alone only durabilises the file data;
            // the new directory entry can remain in pagecache and be lost if
            // the system crashes before the OS flushes it. On POSIX (Linux /
            // macOS) this is required for rename durability. On Windows, NTFS
            // handles this differently and File::sync_all on a directory
            // handle is not meaningful, so we restrict the call to Unix.
            #[cfg(unix)]
            if let Some(parent) = abs_path.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    if let Err(e) = dir.sync_all() {
                        // Best-effort: the rename already committed the file;
                        // a dirent fsync failure is logged but not fatal.
                        log::warn!(
                            "[content_store::atomic] parent dir fsync failed for {:?}: {e}",
                            parent
                        );
                    }
                }
            }
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

/// Ensure the file at `abs_path` contains exactly `full_bytes`, whose body
/// (front-matter excluded) hashes to `body_sha256`.
///
/// This is the write-side half of the content store's integrity contract
/// (#4689): the recorded `content_sha256` must always match the bytes actually
/// on disk. `write_if_new` alone cannot guarantee that — it skips an existing
/// file unconditionally, so a stale/partial pre-existing file at the target
/// path would be left in place while the caller records the *new* body's sha,
/// permanently diverging the DB token from disk.
///
/// Behaviour when the file already exists:
/// - on-disk body sha matches `body_sha256` → idempotent no-op.
/// - mismatch → atomically replaced (temp file + rename over the destination)
///   from `full_bytes`, so the file is never observed missing or partial. At
///   ingest the freshly-composed input is authoritative, so overwriting a
///   drifted on-disk file is the correct reconciliation.
pub fn write_or_replace_body(
    abs_path: &Path,
    full_bytes: &[u8],
    body_sha256: &str,
) -> anyhow::Result<()> {
    if abs_path.exists() {
        let disk_sha = read_body_sha256(abs_path).unwrap_or_default();
        if disk_sha == body_sha256 {
            log::debug!(
                "[content_store::atomic] file already on disk with matching body sha: {}",
                abs_path.display()
            );
            return Ok(());
        }
        log::debug!(
            "[content_store::atomic] on-disk body sha mismatch for {} (disk={disk_sha} new={body_sha256}) — re-staging",
            abs_path.display()
        );
    }
    // Write the replacement to a sibling temp file and atomically rename it over
    // the destination. rename() replaces an existing file atomically on POSIX and
    // NTFS, so — unlike unlink-then-write — the destination is never observed
    // missing or partially written even if the process crashes or the write
    // fails mid-way (a committed DB row can never end up pointing at an absent
    // body file). Post-condition: on success `abs_path` holds exactly
    // `full_bytes`, whose body hashes to `body_sha256`.
    write_via_temp_rename(abs_path, full_bytes)
}

/// Write `bytes` to `abs_path` via a sibling tempfile + fsync + atomic rename,
/// **replacing** any existing file. The rename is atomic on POSIX and NTFS, so
/// the destination is never seen missing or half-written. Parent directories are
/// created on demand and the parent dir entry is fsync'd on Unix for durability.
fn write_via_temp_rename(abs_path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = abs_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| anyhow::anyhow!("create_dir_all {:?}: {e}", parent))?;

    let tmp_path = parent.join(format!(".tmp_{}.md", uuid_v4_hex()));
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| anyhow::anyhow!("create tempfile {:?}: {e}", tmp_path))?;
        // Clean up the temp file on a write/fsync failure too — not just on the
        // rename failure below. content_root is the user-facing vault, so a
        // leaked `.tmp_<uuid>.md` is visible in Obsidian and would accumulate.
        if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("write/fsync tempfile {:?}: {e}", tmp_path));
        }
    }

    if let Err(e) = std::fs::rename(&tmp_path, abs_path) {
        // Keep the old destination intact; only our temp is cleaned up.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(anyhow::anyhow!(
            "rename {:?} -> {:?}: {e}",
            tmp_path,
            abs_path
        ));
    }

    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(parent) {
        if let Err(e) = dir.sync_all() {
            log::warn!(
                "[content_store::atomic] parent dir fsync failed for {:?}: {e}",
                parent
            );
        }
    }
    log::debug!(
        "[content_store::atomic] wrote (replace) {}",
        abs_path.display()
    );
    Ok(())
}

/// A summary that has been written to disk and is ready for SQLite upsert.
#[derive(Debug, Clone)]
pub struct StagedSummary {
    /// Identifier of the summary that was staged.
    pub summary_id: String,
    /// Relative content path (forward-slash, e.g.
    /// `"wiki/summaries/source-slug/L1/id.md"`).
    pub content_path: String,
    /// SHA-256 hex digest over the **body bytes** only (front-matter excluded).
    pub content_sha256: String,
}

/// Write a summary `.md` file to disk and return a [`StagedSummary`] ready for
/// SQLite upsert.
///
/// The relative path is built from the input metadata and the `tree_kind`. The
/// `scope_slug` must already be slugified by the caller. The global tree is a
/// singleton, so its summaries all land under one `global/` folder regardless
/// of the day they cover — no date argument is needed.
///
/// If the file already exists with the same body SHA-256 (idempotent re-stage),
/// the existing `StagedSummary` is returned without rewriting.
pub fn stage_summary(
    content_root: &Path,
    input: &SummaryComposeInput<'_>,
    scope_slug: &str,
) -> anyhow::Result<StagedSummary> {
    stage_summary_with_layout(content_root, input, scope_slug, SummaryDiskLayout::Standard)
}

/// Layout-aware variant of [`stage_summary`]. Document source trees pass a
/// [`SummaryDiskLayout::DocSubtree`] (per-document, versioned) or
/// [`SummaryDiskLayout::Merge`] (cross-document merge tier) so the on-disk
/// vault mirrors the logical tree (`notion` → `docs/<page>/v-<ms>` →
/// `merge`). All other callers use [`stage_summary`] (`Standard`) unchanged.
pub fn stage_summary_with_layout(
    content_root: &Path,
    input: &SummaryComposeInput<'_>,
    scope_slug: &str,
    layout: SummaryDiskLayout<'_>,
) -> anyhow::Result<StagedSummary> {
    let rel_path = summary_rel_path_with_layout(
        input.tree_kind,
        scope_slug,
        input.level,
        input.summary_id,
        layout,
    );
    // Derive the absolute path by joining the relative path components onto
    // the content root (same join `summary_abs_path` does internally) so the
    // two stay consistent regardless of layout.
    let abs_path = {
        let mut abs = content_root.to_path_buf();
        for component in rel_path.split('/') {
            abs.push(component);
        }
        abs
    };

    let composed = compose_summary_md(input);
    let body_bytes = composed.body.as_bytes();
    let sha256 = sha256_hex(body_bytes);

    // Idempotent re-stage that self-heals a stale/corrupt on-disk file: matching
    // body sha is a no-op, a mismatch is atomically rewritten. Not re-writing
    // would leave SQLite storing a content_sha256 that doesn't match the actual
    // on-disk bytes, breaking integrity checks. See [`write_or_replace_body`].
    write_or_replace_body(&abs_path, composed.full.as_bytes(), &sha256)?;

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

/// Read a summary/chunk `.md` file from disk, split off the YAML front-matter,
/// and return the SHA-256 hex digest of the **body bytes only**. Returns an
/// empty string (not an error) if the file cannot be read or parsed, so
/// callers can use the result as a cache key without propagating IO errors.
fn read_body_sha256(path: &Path) -> anyhow::Result<String> {
    let raw = std::fs::read(path)?;
    let content = std::str::from_utf8(&raw)?;
    let (_fm, body) = split_front_matter(content)
        .ok_or_else(|| anyhow::anyhow!("no front-matter in {:?}", path))?;
    Ok(sha256_hex(body.as_bytes()))
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
    use crate::openhuman::memory_store::content::compose::SummaryComposeInput;
    use crate::openhuman::memory_store::content::paths::SummaryTreeKind;
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

    #[test]
    fn write_or_replace_body_writes_new_and_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.md");
        let full = b"---\nk: v\n---\nBODY";
        let body_sha = sha256_hex(b"BODY");
        // Fresh write.
        write_or_replace_body(&path, full, &body_sha).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), full);
        // Idempotent: a matching on-disk body sha leaves the file untouched.
        write_or_replace_body(&path, full, &body_sha).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), full);
    }

    #[test]
    fn write_or_replace_body_overwrites_stale_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.md");
        // A stale pre-existing file with a different body would be skipped by
        // write_if_new; write_or_replace_body must reconcile it (#4689).
        write_if_new(&path, b"---\nk: v\n---\nOLD").unwrap();
        let new_full = b"---\nk: v\n---\nNEW";
        write_or_replace_body(&path, new_full, &sha256_hex(b"NEW")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), new_full);
    }

    #[test]
    fn write_or_replace_body_errors_when_stale_target_cannot_be_replaced() {
        // If the stale target can't be removed/overwritten (here: a directory
        // sits at the path), the function must refuse rather than let the caller
        // record a content_sha256 the on-disk bytes do not match (#4689).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.md");
        std::fs::create_dir_all(&path).unwrap();
        let res = write_or_replace_body(&path, b"---\nk: v\n---\nNEW", &sha256_hex(b"NEW"));
        assert!(
            res.is_err(),
            "must not record a sha the target does not hold"
        );
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
            child_basenames: None,
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
        let staged = stage_summary(dir.path(), &input, "gmail-alice-x-com").unwrap();
        assert_eq!(staged.summary_id, "summary:L1:test1");
        assert!(staged.content_path.starts_with("wiki/summaries/source-"));
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
        let first = stage_summary(dir.path(), &input, "person-alex").unwrap();
        let second = stage_summary(dir.path(), &input, "person-alex").unwrap();
        assert_eq!(first.content_sha256, second.content_sha256);
        assert_eq!(first.content_path, second.content_path);
    }

    #[test]
    fn stage_summary_global_uses_singleton_folder_no_date() {
        let dir = TempDir::new().unwrap();
        let children = vec![];
        let input = mk_summary_input(
            SummaryTreeKind::Global,
            "global",
            "summary:L0:daily",
            "daily recap",
            &children,
        );
        let staged = stage_summary(dir.path(), &input, "global").unwrap();
        // Singleton global tree → one folder, no per-day date segment. The
        // `L1` segment comes from `mk_summary_input`'s level=1; what matters
        // is the single `global/` folder with no date.
        assert_eq!(
            staged.content_path, "wiki/summaries/global/L1/summary-L0-daily.md",
            "global summary must land in the singleton global/ folder; got: {}",
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
        let staged = stage_summary(dir.path(), &input, "gmail-x-y-com").unwrap();
        let expected = sha256_hex(body.as_bytes());
        assert_eq!(staged.content_sha256, expected);
    }

    #[test]
    fn stage_summary_rewrites_stale_on_disk_body() {
        // Create a tempdir and write a "stale" file at the expected path with
        // a body that differs from what the new stage_summary call would write.
        // After stage_summary, the file on disk must match the new body.
        let dir = TempDir::new().unwrap();
        let children = vec!["c1".to_string()];
        let new_body = "fresh body for re-stage test";
        let input = mk_summary_input(
            SummaryTreeKind::Source,
            "gmail:stale@test.com",
            "summary:L1:stale-test",
            new_body,
            &children,
        );

        // First stage with the real body to get the path.
        let first = stage_summary(dir.path(), &input, "gmail-stale-test-com").unwrap();

        // Corrupt the on-disk file by writing a different body to the path.
        let mut abs = dir.path().to_path_buf();
        for part in first.content_path.split('/') {
            abs.push(part);
        }
        // Overwrite with stale content.
        std::fs::write(&abs, b"---\nstale_key: true\n---\nSTALE BODY CONTENT").unwrap();

        // Now re-stage: must detect sha mismatch and re-write.
        let second = stage_summary(dir.path(), &input, "gmail-stale-test-com").unwrap();

        // The returned sha must match the new body.
        let expected_sha = sha256_hex(new_body.as_bytes());
        assert_eq!(
            second.content_sha256, expected_sha,
            "re-staged sha must match new body"
        );

        // The on-disk file must now contain the new body (not the stale one).
        let disk_bytes = std::fs::read(&abs).unwrap();
        let disk_str = std::str::from_utf8(&disk_bytes).unwrap();
        assert!(
            disk_str.contains(new_body),
            "on-disk file must contain new body after re-stage"
        );
        assert!(
            !disk_str.contains("STALE BODY CONTENT"),
            "stale body must be gone after re-stage"
        );
    }
}
