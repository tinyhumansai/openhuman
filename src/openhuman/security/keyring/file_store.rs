//! Crash- and race-safe file primitives shared by the two file keyring backends.
//!
//! [`super::backend::FileBackend`] and [`super::encrypted_file_backend`] both
//! keep every secret in **one** file and mutate it with a read → modify → write
//! cycle. That shape is fine within a process — a mutex covers it — and unsafe
//! across processes, which is the configuration this codebase actually runs in:
//! a desktop core, a `medulla` TUI embedding the same core, and any `cargo test`
//! run that inherits `OPENHUMAN_WORKSPACE` all address the same file.
//!
//! Two failures follow from doing that unguarded, and both destroy secrets
//! rather than merely failing:
//!
//! - **Lost update.** A writes at t0 from a map it read at t-1, silently
//!   discarding B's t-0.5 write. For the app-session entry that reads as "I
//!   signed in, then got signed out again".
//! - **Interleaved temp file.** Both backends staged their write through a
//!   *fixed* sibling path (`dev-keychain.json.tmp` / `secrets.enc.tmp`), so two
//!   writers wrote the same bytes-in-progress and one renamed the other's
//!   half-written buffer into place. The result parses as garbage, and the
//!   plaintext backend used to treat a parse failure as an empty map — so the
//!   next `set` replaced thousands of secrets with one.
//!
//! [`lock_for_write`] closes the first (an advisory whole-file lock held for the
//! full cycle, honoured across processes) and [`write_atomic`] closes the second
//! (a temp name unique to this process and call). Neither is a substitute for
//! the other.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fs2::FileExt;

use crate::openhuman::security::keyring::error::KeyringError;

/// Distinguishes concurrent temp files written by one process.
///
/// The pid alone is not enough: two threads in the same process stage their
/// writes at the same instant, and the lock below is per *file*, not per
/// backend instance, so a caller that legitimately holds no lock (a migration,
/// a quarantine rewrite) can still overlap.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An exclusive advisory lock on a secrets file, released on drop.
///
/// Held on a sidecar `<path>.lock` rather than the secrets file itself, because
/// [`write_atomic`] replaces the secrets file by rename — a lock taken on the
/// old inode would guard nothing once the rename lands.
pub struct WriteLock {
    /// Kept alive purely for its lock; `fs2` releases on close.
    _file: File,
}

/// Take the exclusive write lock covering `path`'s read → modify → write cycle.
///
/// Blocks until the lock is free. Callers must hold the returned guard for the
/// *whole* cycle — acquiring it around the write alone reintroduces the lost
/// update it exists to prevent.
///
/// The lock is advisory and shared between threads as well as processes (`flock`
/// and `LockFileEx` both serialize independent handles regardless of origin).
///
/// # Errors
///
/// Returns [`KeyringError::Backend`] when the lock file's directory cannot be
/// created, the lock file cannot be opened, or the lock cannot be taken. Failing
/// the operation is deliberate: proceeding unlocked is what loses secrets.
pub fn lock_for_write(path: &Path) -> Result<WriteLock, KeyringError> {
    let lock_path = lock_path_for(path);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            KeyringError::Backend(format!(
                "could not create {} for the keyring lock: {e}",
                parent.display()
            ))
        })?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| {
            KeyringError::Backend(format!(
                "could not open the keyring lock at {}: {e}",
                lock_path.display()
            ))
        })?;
    file.lock_exclusive().map_err(|e| {
        KeyringError::Backend(format!(
            "could not lock the keyring at {}: {e}",
            lock_path.display()
        ))
    })?;
    Ok(WriteLock { _file: file })
}

/// The sidecar lock path for a secrets file.
pub fn lock_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

/// Replace `path`'s contents with `bytes`, atomically and `0600`.
///
/// Staged through a temp file unique to this process and call, then renamed —
/// so a concurrent writer can never observe, or rename into place, a partially
/// written buffer. The temp file is removed if the rename fails, leaving no
/// debris behind for the next run to trip over.
///
/// # Errors
///
/// Returns [`KeyringError::Backend`] when the parent directory cannot be
/// created, or the temp file cannot be written or renamed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), KeyringError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            KeyringError::Backend(format!(
                "could not create {} for a keyring write: {e}",
                parent.display()
            ))
        })?;
    }

    // A stale temp file can survive a crash. PID reuse then makes the first
    // sequence value collide, so keep allocating sequence values until a new
    // staging path is reserved for this write.
    let (tmp_path, mut file) = reserve_temp_file(|| temp_path_for(path))?;
    let mut write = || -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Before any bytes land: a window where the file is world-readable
            // is a window where a secret is world-readable.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        // The rename below is atomic with respect to *ordering*, not durability.
        // Without this a crash can leave the renamed file present but empty.
        file.sync_all()
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        KeyringError::Backend(format!(
            "could not stage a keyring write at {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        KeyringError::Backend(format!(
            "could not replace the keyring file at {}: {e}",
            path.display()
        ))
    })
}

/// Reserve a fresh temp file, advancing past leftovers from crashed writers.
fn reserve_temp_file(
    mut next_path: impl FnMut() -> PathBuf,
) -> Result<(PathBuf, File), KeyringError> {
    loop {
        let tmp_path = next_path();
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
        {
            Ok(file) => break Ok((tmp_path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(KeyringError::Backend(format!(
                    "could not stage a keyring write at {}: {e}",
                    tmp_path.display()
                )));
            }
        }
    }
}

/// A temp sibling of `path` that no other process or thread will pick.
fn temp_path_for(path: &Path) -> PathBuf {
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{seq}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// Move a file that cannot be parsed aside, so its bytes survive for recovery.
///
/// Returns the quarantine path when the file was moved. A failure to move is
/// logged and reported as `None` rather than propagated: the caller is already
/// on an error path, and losing the quarantine is not worse than the corruption
/// that triggered it.
///
/// `suffix` names the artefact (`"json"`, `"enc"`) so the quarantined file keeps
/// a recognisable extension.
pub fn quarantine_corrupt(path: &Path, suffix: &str) -> Option<PathBuf> {
    let stamp = chrono::Utc::now().timestamp();
    let target = path.with_extension(format!("{suffix}.corrupt.{stamp}"));
    match std::fs::rename(path, &target) {
        Ok(()) => {
            log::error!(
                "[keyring] {} could not be parsed; moved to {} and treated as empty",
                path.display(),
                target.display()
            );
            Some(target)
        }
        Err(e) => {
            log::error!(
                "[keyring] {} could not be parsed and could not be moved aside: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "file_store_tests.rs"]
mod tests;
