//! Read / edit / reset for the bundled persona prompt files (`SOUL.md`,
//! `IDENTITY.md`) that drive the agent's personality.
//!
//! Backs the Persona Pack settings surface (issue #2345). The editable set is
//! restricted to the bundled bootstrap files (see
//! [`crate::openhuman::workspace::ops::bundled_default_contents`]) so a caller
//! can never read or overwrite an arbitrary path under the workspace.

use std::path::Path;

use serde::Serialize;

use crate::openhuman::workspace::ops::bundled_default_contents;
use crate::rpc::RpcOutcome;

/// Hard cap on the size accepted by [`write_workspace_file`]. `SOUL.md` /
/// `IDENTITY.md` are prose prompts measured in kilobytes; the cap exists purely
/// so a runaway paste cannot balloon the workspace or the prompt-injection
/// budget the files are later spliced into.
pub const MAX_WORKSPACE_FILE_BYTES: usize = 256 * 1024;

/// A single editable persona file plus the metadata the settings UI needs to
/// render and round-trip it.
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
    /// Absolute path the contents map to on disk.
    pub path: String,
}

/// Resolve the bundled default for `filename`, rejecting any name that is not
/// part of the editable allowlist.
fn ensure_editable(filename: &str) -> Result<&'static str, String> {
    bundled_default_contents(filename)
        .ok_or_else(|| format!("'{filename}' is not an editable workspace file"))
}

/// Read an editable persona file. When the workspace copy is missing (e.g. a
/// fresh install that has not run `init` yet) the bundled default is returned
/// with `is_default = true` so the editor always shows the effective prompt.
pub fn read_workspace_file(
    workspace_dir: &Path,
    filename: &str,
) -> Result<RpcOutcome<WorkspaceFile>, String> {
    let default_contents = ensure_editable(filename)?;
    let path = workspace_dir.join(filename);
    let (contents, is_default) = match std::fs::read_to_string(&path) {
        Ok(text) => (text, false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (default_contents.to_string(), true),
        Err(e) => return Err(format!("failed to read {}: {e}", path.display())),
    };
    log::debug!(
        "[workspace][rpc] read file='{filename}' is_default={is_default} bytes={}",
        contents.len()
    );
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents,
            is_default,
            path: path.display().to_string(),
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
    if contents.len() > MAX_WORKSPACE_FILE_BYTES {
        return Err(format!(
            "contents for {filename} exceed the {MAX_WORKSPACE_FILE_BYTES}-byte limit"
        ));
    }
    std::fs::create_dir_all(workspace_dir).map_err(|e| {
        format!(
            "failed to create workspace dir {}: {e}",
            workspace_dir.display()
        )
    })?;
    let path = workspace_dir.join(filename);
    std::fs::write(&path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    log::debug!(
        "[workspace][rpc] wrote file='{filename}' bytes={}",
        contents.len()
    );
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents: contents.to_string(),
            is_default: false,
            path: path.display().to_string(),
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
    std::fs::create_dir_all(workspace_dir).map_err(|e| {
        format!(
            "failed to create workspace dir {}: {e}",
            workspace_dir.display()
        )
    })?;
    let path = workspace_dir.join(filename);
    std::fs::write(&path, default_contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    log::debug!("[workspace][rpc] reset file='{filename}' to bundled default");
    Ok(RpcOutcome::new(
        WorkspaceFile {
            filename: filename.to_string(),
            contents: default_contents.to_string(),
            is_default: true,
            path: path.display().to_string(),
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
        let huge = "a".repeat(MAX_WORKSPACE_FILE_BYTES + 1);
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
        let at_limit = "a".repeat(MAX_WORKSPACE_FILE_BYTES);
        let written = write_workspace_file(tmp.path(), "SOUL.md", &at_limit)
            .expect("exactly-at-limit write should succeed")
            .value;
        assert_eq!(written.contents.len(), MAX_WORKSPACE_FILE_BYTES);
    }
}
