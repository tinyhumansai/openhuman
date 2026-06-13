//! Read / edit / reset for the bundled persona prompt files (`SOUL.md`,
//! `IDENTITY.md`) that drive the agent's personality.
//!
//! Backs the Persona Pack settings surface (issue #2345). The editable set is
//! restricted to the bundled bootstrap files (see
//! [`crate::openhuman::workspace::ops::bundled_default_contents`]) so a caller
//! can never read or overwrite an arbitrary path under the workspace.

use std::io::Read;
use std::path::Path;

use serde::Serialize;

use crate::openhuman::workspace::ops::bundled_default_contents;
use crate::rpc::RpcOutcome;

/// Hard cap on the size accepted by [`write_workspace_file`] and tolerated by
/// [`read_workspace_file`]. `SOUL.md` / `IDENTITY.md` are prose prompts
/// measured in kilobytes; the cap bounds both a runaway paste from the UI and a
/// pathologically large file dropped on disk by another process (which would
/// otherwise be slurped fully into memory and shipped over RPC).
pub const MAX_WORKSPACE_FILE_BYTES: u64 = 256 * 1024;

/// A single editable persona file plus the metadata the settings UI needs to
/// render and round-trip it. The absolute on-disk path is deliberately **not**
/// part of this payload — the UI never needs it, and returning it would leak
/// host filesystem layout to every RPC caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceFile {
    /// Allowlisted file name (e.g. `SOUL.md`).
    pub filename: String,
    /// Current effective contents.
    pub contents: String,
    /// `true` when `contents` came from the bundled default rather than a file
    /// on disk — i.e. the workspace copy was missing (read) or has just been
    /// restored (reset). Lets the UI show a "using default" affordance.
    pub is_default: bool,
}

/// Resolve the bundled default for `filename`, rejecting any name that is not
/// part of the editable allowlist.
fn ensure_editable(filename: &str) -> Result<&'static str, String> {
    bundled_default_contents(filename).ok_or_else(|| {
        log::debug!("[workspace][rpc] rejected non-editable filename='{filename}'");
        format!("'{filename}' is not an editable workspace file")
    })
}

/// Read an editable persona file. When the workspace copy is missing (e.g. a
/// fresh install that has not run `init` yet) the bundled default is returned
/// with `is_default = true` so the editor always shows the effective prompt.
/// A file larger than [`MAX_WORKSPACE_FILE_BYTES`] is refused rather than
/// loaded into memory.
pub fn read_workspace_file(
    workspace_dir: &Path,
    filename: &str,
) -> Result<RpcOutcome<WorkspaceFile>, String> {
    let default_contents = ensure_editable(filename)?;
    let path = workspace_dir.join(filename);

    // Open first, then enforce the cap on the *opened handle* via a bounded
    // reader. A prior `metadata().len()` check would be TOCTOU-prone: another
    // process could grow or swap the file between the stat and the read, so an
    // oversized file could still be slurped fully into memory. Reading through
    // `take(cap + 1)` caps the bytes we will ever hold regardless of races.
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "[workspace][rpc] read fallback-to-default file='{filename}' (missing on disk)"
            );
            return Ok(RpcOutcome::new(
                WorkspaceFile {
                    filename: filename.to_string(),
                    contents: default_contents.to_string(),
                    is_default: true,
                },
                Vec::new(),
            ));
        }
        Err(e) => {
            log::debug!(
                "[workspace][rpc] read open-failed file='{filename}' path='{}': {e}",
                path.display()
            );
            return Err(format!("failed to read {filename}: {e}"));
        }
    };

    let mut buf = Vec::new();
    file.take(MAX_WORKSPACE_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| {
            log::debug!(
                "[workspace][rpc] read failed file='{filename}' path='{}': {e}",
                path.display()
            );
            format!("failed to read {filename}: {e}")
        })?;
    // Hitting the extra byte means the file exceeds the cap; refuse it.
    if buf.len() as u64 > MAX_WORKSPACE_FILE_BYTES {
        log::debug!(
            "[workspace][rpc] read refused file='{filename}' (over cap={MAX_WORKSPACE_FILE_BYTES})"
        );
        return Err(format!(
            "{filename} is too large to edit (over the {MAX_WORKSPACE_FILE_BYTES}-byte limit)"
        ));
    }

    let contents = String::from_utf8(buf).map_err(|e| {
        log::debug!("[workspace][rpc] read rejected non-utf8 file='{filename}': {e}");
        format!("failed to read {filename}: contents are not valid UTF-8")
    })?;
    log::debug!(
        "[workspace][rpc] read ok file='{filename}' bytes={}",
        contents.len()
    );
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents,
            is_default: false,
        },
        Vec::new(),
    ))
}

/// Overwrite an editable persona file with user-supplied contents. Rejects
/// non-allowlisted names and anything over [`MAX_WORKSPACE_FILE_BYTES`]; the
/// workspace directory is created if it does not yet exist.
pub fn write_workspace_file(
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
) -> Result<RpcOutcome<WorkspaceFile>, String> {
    ensure_editable(filename)?;
    if contents.len() as u64 > MAX_WORKSPACE_FILE_BYTES {
        log::debug!(
            "[workspace][rpc] write refused file='{filename}' bytes={} cap={MAX_WORKSPACE_FILE_BYTES}",
            contents.len()
        );
        return Err(format!(
            "contents for {filename} exceed the {MAX_WORKSPACE_FILE_BYTES}-byte limit"
        ));
    }
    let path = workspace_dir.join(filename);
    std::fs::create_dir_all(workspace_dir).map_err(|e| {
        log::debug!(
            "[workspace][rpc] write mkdir-failed dir='{}': {e}",
            workspace_dir.display()
        );
        format!("failed to prepare the workspace directory: {e}")
    })?;
    std::fs::write(&path, contents).map_err(|e| {
        log::debug!(
            "[workspace][rpc] write failed file='{filename}' path='{}': {e}",
            path.display()
        );
        format!("failed to write {filename}: {e}")
    })?;
    log::debug!(
        "[workspace][rpc] write ok file='{filename}' bytes={}",
        contents.len()
    );
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents: contents.to_string(),
            is_default: false,
        },
        Vec::new(),
    ))
}

/// Restore an editable persona file to its bundled default and return the
/// restored contents.
pub fn reset_workspace_file(
    workspace_dir: &Path,
    filename: &str,
) -> Result<RpcOutcome<WorkspaceFile>, String> {
    let default_contents = ensure_editable(filename)?;
    let path = workspace_dir.join(filename);
    std::fs::create_dir_all(workspace_dir).map_err(|e| {
        log::debug!(
            "[workspace][rpc] reset mkdir-failed dir='{}': {e}",
            workspace_dir.display()
        );
        format!("failed to prepare the workspace directory: {e}")
    })?;
    std::fs::write(&path, default_contents).map_err(|e| {
        log::debug!(
            "[workspace][rpc] reset failed file='{filename}' path='{}': {e}",
            path.display()
        );
        format!("failed to reset {filename}: {e}")
    })?;
    log::debug!("[workspace][rpc] reset ok file='{filename}' (restored bundled default)");
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents: default_contents.to_string(),
            is_default: true,
        },
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_returns_bundled_default_when_file_missing() {
        let tmp = tempdir().unwrap();
        let outcome = read_workspace_file(tmp.path(), "SOUL.md").expect("read should succeed");
        let file = outcome.value;
        assert!(file.is_default, "missing file should report the default");
        assert!(!file.contents.trim().is_empty());
        assert_eq!(file.filename, "SOUL.md");
        assert_eq!(
            file.contents,
            bundled_default_contents("SOUL.md").unwrap(),
            "default read must match the bundled prompt"
        );
    }

    #[test]
    fn read_returns_on_disk_contents_when_present() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("SOUL.md"), "custom soul").unwrap();
        let file = read_workspace_file(tmp.path(), "SOUL.md")
            .expect("read ok")
            .value;
        assert!(!file.is_default);
        assert_eq!(file.contents, "custom soul");
    }

    #[test]
    fn read_refuses_oversize_file_on_disk() {
        let tmp = tempdir().unwrap();
        let huge = "a".repeat((MAX_WORKSPACE_FILE_BYTES + 1) as usize);
        std::fs::write(tmp.path().join("SOUL.md"), &huge).unwrap();
        let err = read_workspace_file(tmp.path(), "SOUL.md").unwrap_err();
        assert!(err.contains("too large"), "unexpected error: {err}");
    }

    #[test]
    fn read_accepts_file_exactly_at_the_size_limit() {
        let tmp = tempdir().unwrap();
        let at_limit = "a".repeat(MAX_WORKSPACE_FILE_BYTES as usize);
        std::fs::write(tmp.path().join("SOUL.md"), &at_limit).unwrap();
        let file = read_workspace_file(tmp.path(), "SOUL.md")
            .expect("exactly-at-limit read should succeed")
            .value;
        assert_eq!(file.contents.len(), MAX_WORKSPACE_FILE_BYTES as usize);
        assert!(!file.is_default);
    }

    #[test]
    fn read_rejects_non_utf8_file() {
        let tmp = tempdir().unwrap();
        // A lone 0xFF byte is never valid UTF-8.
        std::fs::write(tmp.path().join("SOUL.md"), [0xff_u8, 0xfe, 0xfd]).unwrap();
        let err = read_workspace_file(tmp.path(), "SOUL.md").unwrap_err();
        assert!(err.contains("UTF-8"), "unexpected error: {err}");
    }

    #[test]
    fn write_then_read_round_trips() {
        let tmp = tempdir().unwrap();
        let written = write_workspace_file(tmp.path(), "SOUL.md", "You are calm and concise.")
            .expect("write ok")
            .value;
        assert!(!written.is_default);
        assert_eq!(written.contents, "You are calm and concise.");

        let read = read_workspace_file(tmp.path(), "SOUL.md")
            .expect("read ok")
            .value;
        assert_eq!(read.contents, "You are calm and concise.");
        assert!(!read.is_default);
    }

    #[test]
    fn write_creates_workspace_dir_if_missing() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("does/not/exist/yet");
        let written = write_workspace_file(&nested, "IDENTITY.md", "id")
            .expect("write should create the dir")
            .value;
        assert_eq!(written.contents, "id");
        assert!(nested.join("IDENTITY.md").is_file());
    }

    #[test]
    fn reset_restores_bundled_default() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("SOUL.md"), "corrupted").unwrap();
        let reset = reset_workspace_file(tmp.path(), "SOUL.md")
            .expect("reset ok")
            .value;
        assert!(reset.is_default);
        assert_eq!(reset.contents, bundled_default_contents("SOUL.md").unwrap());
        let on_disk = std::fs::read_to_string(tmp.path().join("SOUL.md")).unwrap();
        assert_eq!(on_disk, bundled_default_contents("SOUL.md").unwrap());
    }

    #[test]
    fn non_allowlisted_filename_is_rejected_for_every_op() {
        let tmp = tempdir().unwrap();
        for name in ["secrets.txt", "../escape.md", "MEMORY.md", "soul.md"] {
            assert!(read_workspace_file(tmp.path(), name).is_err());
            assert!(write_workspace_file(tmp.path(), name, "x").is_err());
            assert!(reset_workspace_file(tmp.path(), name).is_err());
        }
        // The rejection must not have written anything to disk.
        assert!(!tmp.path().join("MEMORY.md").exists());
    }

    #[test]
    fn write_rejects_oversize_contents() {
        let tmp = tempdir().unwrap();
        let huge = "a".repeat((MAX_WORKSPACE_FILE_BYTES + 1) as usize);
        let err = write_workspace_file(tmp.path(), "SOUL.md", &huge).unwrap_err();
        assert!(err.contains("limit"), "unexpected error: {err}");
        assert!(
            !tmp.path().join("SOUL.md").exists(),
            "oversize write must not touch disk"
        );
    }

    #[test]
    fn write_accepts_contents_at_the_size_limit() {
        let tmp = tempdir().unwrap();
        let at_limit = "a".repeat(MAX_WORKSPACE_FILE_BYTES as usize);
        let written = write_workspace_file(tmp.path(), "SOUL.md", &at_limit)
            .expect("exactly-at-limit write should succeed")
            .value;
        assert_eq!(written.contents.len(), MAX_WORKSPACE_FILE_BYTES as usize);
    }
}
