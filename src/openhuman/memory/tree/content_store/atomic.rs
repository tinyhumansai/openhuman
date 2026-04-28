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
}
