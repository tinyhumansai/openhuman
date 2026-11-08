//! Tightening of an owned secrets file left looser than `0600`.
//!
//! The TinyDrivers secrets driver admits an existing file only when it is a
//! regular, single-link file owned by this user at exactly `0600`, and refuses
//! anything else. An older build, a copy or a restore can leave an *owned* file
//! wider than that, and refusing it would make every rewrite of a token, lease
//! or lock fail until someone ran `chmod`. The repair is narrow: only a file this
//! user already owns, opened without following symlinks, is tightened, and the
//! driver still performs its own admission afterwards. Anything it would
//! refuse for another reason (foreign owner, symlink, hard link, non-regular
//! file) is left for it to refuse.

use std::path::Path;

/// Tighten `path` to `0600` when it is a regular file this user owns with a
/// wider mode. A missing file, or one that is not ours to repair, is left alone.
#[cfg(all(unix, feature = "secret-file"))]
pub(super) fn tighten_owned_file(path: &Path) {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            tracing::trace!(
                path = %path.display(),
                kind = ?error.kind(),
                "[secret_storage] no owned file to tighten"
            );
            return;
        }
    };
    let Ok(metadata) = file.metadata() else {
        return;
    };
    // SAFETY: `geteuid` takes no arguments and mutates no process state.
    let euid = unsafe { libc::geteuid() };
    if !metadata.is_file() || metadata.uid() != euid || metadata.mode() & 0o777 == 0o600 {
        return;
    }
    match file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => tracing::debug!(
            path = %path.display(),
            from_mode = format!("{:o}", metadata.mode() & 0o777),
            "[secret_storage] tightened an owned secrets file to 0600"
        ),
        Err(error) => tracing::warn!(
            path = %path.display(),
            %error,
            "[secret_storage] could not tighten an owned secrets file"
        ),
    }
}

#[cfg(not(all(unix, feature = "secret-file")))]
pub(super) fn tighten_owned_file(_path: &Path) {}
