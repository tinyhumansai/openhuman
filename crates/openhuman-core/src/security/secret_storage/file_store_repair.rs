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

/// Open `path` read-only without following a final symlink, for `fchmod` and
/// `fstat` only. No write handle is ever taken: the repair changes the mode,
/// never the bytes.
#[cfg(all(unix, feature = "secret-file"))]
fn open_no_follow(path: &Path, extra: libc::c_int) -> std::io::Result<std::fs::File> {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | extra,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned by `open` and is owned by nothing else.
    Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

/// Tighten `path` to `0600` when it is a regular file this user owns with a
/// wider mode. A missing file, or one that is not ours to repair, is left alone.
#[cfg(all(unix, feature = "secret-file"))]
pub(super) fn tighten_owned_file(path: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let file = match open_no_follow(path, libc::O_NONBLOCK) {
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
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let Ok(file) = open_no_follow(dir, libc::O_DIRECTORY) else {
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
