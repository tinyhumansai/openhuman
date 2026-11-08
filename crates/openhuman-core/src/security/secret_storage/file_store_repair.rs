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
    // A file others could write may hold content the core never wrote:
    // tightening it would turn a possible forgery into a trusted secret. Leave
    // it for the driver to refuse. Only a leaked *read* (group/other read
    // bits) is repaired.
    if metadata.mode() & 0o022 != 0 {
        tracing::warn!(
            path = %path.display(),
            mode = format!("{:o}", metadata.mode() & 0o777),
            "[secret_storage] an owned secrets file is writable by others; not repairing it"
        );
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

/// Remove group and other write from the directory that holds a secrets file,
/// when this user owns it.
///
/// The driver refuses a secrets namespace another user could mutate. A directory
/// the core created itself under a permissive umask (`0002` is the default on
/// many Linux distributions, which makes it `0775`) is ours to repair; leaving it
/// would make every secret read and write fail. Only the leaf is touched: an
/// ancestor that is foreign-writable is not ours, and the driver still refuses it.
#[cfg(all(unix, feature = "secret-file"))]
pub(super) fn tighten_owned_directory(dir: &Path) {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(dir)
    else {
        return;
    };
    let Ok(metadata) = file.metadata() else {
        return;
    };
    // SAFETY: `geteuid` takes no arguments and mutates no process state.
    let euid = unsafe { libc::geteuid() };
    let mode = metadata.mode() & 0o7777;
    if !metadata.is_dir() || metadata.uid() != euid || mode & 0o022 == 0 {
        return;
    }
    match file.set_permissions(std::fs::Permissions::from_mode(mode & !0o022)) {
        Ok(()) => tracing::debug!(
            path = %dir.display(),
            from_mode = format!("{mode:o}"),
            "[secret_storage] removed group/other write from an owned secrets directory"
        ),
        Err(error) => tracing::warn!(
            path = %dir.display(),
            %error,
            "[secret_storage] could not tighten an owned secrets directory"
        ),
    }
}

#[cfg(not(all(unix, feature = "secret-file")))]
pub(super) fn tighten_owned_directory(_dir: &Path) {}
