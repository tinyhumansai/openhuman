//! Crash- and race-safe file primitives shared by the two file keyring backends.
//!
//! [`super::backend::FileBackend`] and [`super::encrypted_file_backend`] both
//! keep every secret in **one** file and mutate it with a read → modify → write
//! cycle. That shape is fine within a process — a mutex covers it — and unsafe
//! across processes, which is the configuration this codebase actually runs in:
//! a desktop core, another process embedding the same core, and any `cargo test`
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
use std::io::Read;
#[cfg(not(feature = "secret-file"))]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::security::secret_storage::error::SecretStorageError;

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
    _lock: LockGuard,
}

enum LockGuard {
    #[cfg(feature = "secret-file")]
    Driver(tinystoragedrivers::secrets::Locked),
    #[cfg(not(feature = "secret-file"))]
    Native(File),
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
/// Returns [`SecretStorageError::Backend`] when the lock file's directory cannot be
/// created, the lock file cannot be opened, or the lock cannot be taken. Failing
/// the operation is deliberate: proceeding unlocked is what loses secrets.
pub fn lock_for_write(path: &Path) -> Result<WriteLock, SecretStorageError> {
    let lock_path = lock_path_for(path);
    #[cfg(feature = "secret-file")]
    {
        tinystoragedrivers::secrets::lock_file(&lock_path)
            .map(|lock| WriteLock {
                _lock: LockGuard::Driver(lock),
            })
            .map_err(|error| {
                SecretStorageError::Backend(format!("could not lock keyring: {error}"))
            })
    }
    #[cfg(not(feature = "secret-file"))]
    {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                SecretStorageError::Backend(format!(
                    "could not create {} for the keyring lock: {e}",
                    parent.display()
                ))
            })?;
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&lock_path).map_err(|e| {
            SecretStorageError::Backend(format!(
                "could not open the keyring lock at {}: {e}",
                lock_path.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = file.metadata().map_err(|error| {
                SecretStorageError::Backend(format!(
                    "could not inspect the keyring lock at {}: {error}",
                    lock_path.display()
                ))
            })?;
            // A lock owned by another user is not authority over this store. Old
            // OpenHuman lock files may have inherited wider modes, so tighten an
            // owned regular file before the stricter TinyDrivers sidecar opens it.
            if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(SecretStorageError::Backend(format!(
                    "keyring lock at {} is not a regular file owned by this user",
                    lock_path.display()
                )));
            }
            if metadata.permissions().mode() & 0o777 != 0o600 {
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .map_err(|error| {
                        SecretStorageError::Backend(format!(
                            "could not restrict permissions on the keyring lock at {}: {error}",
                            lock_path.display()
                        ))
                    })?;
            }
        }
        fs2::FileExt::lock_exclusive(&file).map_err(|e| {
            SecretStorageError::Backend(format!(
                "could not lock the keyring at {}: {e}",
                lock_path.display()
            ))
        })?;
        Ok(WriteLock {
            _lock: LockGuard::Native(file),
        })
    }
}

/// The sidecar lock path for a secrets file.
pub fn lock_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

/// Fingerprint the current regular file without retaining its contents.
///
/// Callers use this around a separately locked read to detect whether another
/// writer replaced the file before they acquire [`lock_for_write`].
pub fn fingerprint(path: &Path) -> Result<Option<[u8; 32]>, SecretStorageError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(SecretStorageError::Backend(format!(
                "could not inspect {} for the keyring: {error}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(SecretStorageError::Backend(format!(
            "refusing to fingerprint a non-regular keyring file at {}",
            path.display()
        )));
    }

    let mut file = File::open(path).map_err(|error| {
        SecretStorageError::Backend(format!(
            "could not read {} for the keyring: {error}",
            path.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            SecretStorageError::Backend(format!(
                "could not fingerprint {} for the keyring: {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Some(hasher.finalize().into()))
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Replace `path`'s contents with `bytes`, atomically, durably, and `0600`.
///
/// Staged through a temp file unique to this process and call, then renamed —
/// so a concurrent writer can never observe, or rename into place, a partially
/// written buffer. The temp file is synced before publication, and the rename
/// is made durable before returning, so a verified encrypted migration can
/// then remove its plaintext source. The temp file is removed if publication
/// fails, leaving no debris behind for the next run to trip over.
///
/// # Errors
///
/// Returns [`SecretStorageError::Backend`] when the parent directory cannot be
/// created, or the temp file cannot be written or renamed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SecretStorageError> {
    #[cfg(feature = "secret-file")]
    {
        tinystoragedrivers::secrets::write_atomic_private(path, bytes)
            .map_err(|error| SecretStorageError::Backend(format!("atomic keyring write: {error}")))
    }
    #[cfg(not(feature = "secret-file"))]
    write_atomic_fallback(path, bytes)
}

#[cfg(not(feature = "secret-file"))]
fn write_atomic_fallback(path: &Path, bytes: &[u8]) -> Result<(), SecretStorageError> {
    let parent = parent_dir(path);
    std::fs::create_dir_all(parent).map_err(|e| {
        SecretStorageError::Backend(format!(
            "could not create {} for a keyring write: {e}",
            parent.display()
        ))
    })?;

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
        SecretStorageError::Backend(format!(
            "could not stage a keyring write at {}: {e}",
            tmp_path.display()
        ))
    })?;

    replace_durably(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        SecretStorageError::Backend(format!(
            "could not replace the keyring file at {}: {e}",
            path.display()
        ))
    })
}

/// Publish the synced file and persist its directory entry before callers may
/// remove a plaintext migration source. Unix requires syncing the parent
/// directory after rename; Windows can request a write-through move.
#[cfg(unix)]
fn replace_durably(source: &Path, destination: &Path) -> std::io::Result<()> {
    replace_durably_with_sync(source, destination, |parent| File::open(parent)?.sync_all())
}

/// Ensure a previously published encrypted file is durable before cleaning a
/// matching plaintext migration source. The caller must hold `destination_lock`
/// across the read, comparison, and cleanup; on Windows this also prevents a
/// durability retry from republishing stale bytes over a newer write.
///
/// This retries an earlier publication that returned an error after the
/// destination had already appeared.
pub fn sync_parent_dir(
    path: &Path,
    destination_lock: &WriteLock,
) -> Result<(), SecretStorageError> {
    #[cfg(unix)]
    {
        let _ = destination_lock;
        let parent = parent_dir(path);
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                SecretStorageError::Backend(format!(
                    "could not sync keyring directory {}: {error}",
                    parent.display()
                ))
            })
    }
    #[cfg(windows)]
    {
        // A failed MoveFileExW can leave the destination visible. Republish
        // the same bytes with WRITE_THROUGH and verify the result before the
        // caller deletes the last plaintext copy.
        sync_parent_dir_with_republish(path, destination_lock, write_atomic)
    }
}

#[cfg(windows)]
fn sync_parent_dir_with_republish(
    path: &Path,
    _destination_lock: &WriteLock,
    republish: impl FnOnce(&Path, &[u8]) -> Result<(), SecretStorageError>,
) -> Result<(), SecretStorageError> {
    let expected =
        std::fs::read(path).map_err(|source| SecretStorageError::MigrationReadFailed {
            path: path.display().to_string(),
            source,
        })?;
    republish(path, &expected)?;
    let actual = std::fs::read(path).map_err(|source| SecretStorageError::MigrationReadFailed {
        path: path.display().to_string(),
        source,
    })?;
    if actual != expected {
        return Err(SecretStorageError::Backend(format!(
            "durable keyring publication at {} failed verification",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn replace_durably_with_sync(
    source: &Path,
    destination: &Path,
    sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    std::fs::rename(source, destination)?;
    let parent = parent_dir(destination);
    sync_parent(parent)
}

#[cfg(windows)]
fn replace_durably(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // A directory handle cannot be flushed portably on Windows. The
    // write-through move is the platform's durability barrier for the rename.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
compile_error!("keyring durable replacement requires a platform implementation");

/// Reserve a fresh temp file, advancing past leftovers from crashed writers.
fn reserve_temp_file(
    mut next_path: impl FnMut() -> PathBuf,
) -> Result<(PathBuf, File), SecretStorageError> {
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
                return Err(SecretStorageError::Backend(format!(
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
    let mut attempt = 0u32;
    loop {
        let target = if attempt == 0 {
            path.with_extension(format!("{suffix}.corrupt.{stamp}"))
        } else {
            path.with_extension(format!("{suffix}.corrupt.{stamp}.{attempt}"))
        };
        attempt += 1;
        // `hard_link` refuses an existing destination, so the name is
        // reserved atomically: two processes quarantining in the same second
        // can never overwrite each other's preserved bytes (a `rename` would
        // replace the destination on every platform).
        match std::fs::hard_link(path, &target) {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(path) {
                    // Not moved aside after all: drop the extra link so the
                    // caller does not treat the original as gone.
                    let _ = std::fs::remove_file(&target);
                    log::error!(
                        "[keyring] {} could not be parsed and could not be moved aside: {e}",
                        path.display()
                    );
                    return None;
                }
                return Some(log_quarantined(path, target));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(link_error) => {
                if !path.exists() {
                    log::error!(
                        "[keyring] {} could not be parsed and could not be moved aside: {link_error}",
                        path.display()
                    );
                    return None;
                }
                // Filesystems without hard links: fall back to a checked rename.
                if target.exists() {
                    continue;
                }
                return match std::fs::rename(path, &target) {
                    Ok(()) => Some(log_quarantined(path, target)),
                    Err(e) => {
                        log::error!(
                            "[keyring] {} could not be parsed and could not be moved aside: {e}",
                            path.display()
                        );
                        None
                    }
                };
            }
        }
    }
}

fn log_quarantined(path: &Path, target: PathBuf) -> PathBuf {
    log::error!(
        "[keyring] {} could not be parsed; moved to {} and treated as empty",
        path.display(),
        target.display()
    );
    target
}

#[cfg(test)]
#[path = "file_store_tests.rs"]
mod tests;
