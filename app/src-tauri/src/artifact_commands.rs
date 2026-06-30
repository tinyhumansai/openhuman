//! Tauri commands for exporting agent-generated artifacts (#2779, #3162).
//!
//! Two export paths, both fed by the frontend resolving an artifact's
//! absolute source path via the `openhuman.ai_get_artifact` core RPC:
//!
//! 1. [`save_artifact_via_dialog`] (#3162) — opens a native Save-As
//!    dialog (macOS / Windows / Linux) pre-filled with the artifact's
//!    filename and copies the bytes to the user-chosen destination.
//!    Returns `Ok(None)` when the user cancels. Backed by the `rfd`
//!    crate, which talks to the OS dialog APIs directly and does NOT
//!    pull `tauri-plugin-fs` (whose `schemars` version conflict was the
//!    reason the original #2779 work shipped the Downloads fallback
//!    below instead of a dialog).
//! 2. [`download_artifact_to_downloads`] (#2779) — copies the artifact
//!    into the user's Downloads directory with a non-colliding name and
//!    returns the dest path so the UI can offer "Reveal in Finder".
//!    Retained as the fallback the frontend uses when the dialog is
//!    unavailable (e.g. no portal on headless Linux) or the user cancels.
//!    Cross-platform — previously macOS/Linux-only, un-gated so the
//!    Save-As fallback works on Windows too.
//!
//! Both validate that the source is an existing file inside the
//! OpenHuman data dir's `artifacts/` tree, and sanitize the filename
//! hint, so the renderer can never copy an arbitrary local file out nor
//! write outside the chosen directory.

use std::path::{Path, PathBuf};

/// Open a native Save-As dialog pre-filled with `suggested_filename` and
/// copy the artifact at `source_path` to the chosen destination (#3162).
///
/// Returns:
/// - `Ok(Some(dest))` — the absolute path the user saved to.
/// - `Ok(None)` — the user dismissed the dialog (not an error; the
///   frontend simply stops).
/// - `Err(_)` — bad inputs or a copy failure; the frontend falls back to
///   [`download_artifact_to_downloads`] where available.
#[tauri::command]
pub async fn save_artifact_via_dialog(
    source_path: String,
    suggested_filename: String,
) -> Result<Option<String>, String> {
    let source = validate_source(&source_path)?;
    let sanitized = sanitize_filename(&suggested_filename)?;

    // `rfd` drives the OS-native dialog. On Linux this is the xdg-desktop
    // portal (no GTK link); on macOS/Windows the system panel. The await
    // resolves when the user picks a path or cancels.
    let handle = rfd::AsyncFileDialog::new()
        .set_file_name(&sanitized)
        .save_file()
        .await;

    let Some(file) = handle else {
        log::info!("[artifact_commands] save_artifact_via_dialog cancelled by user");
        return Ok(None);
    };

    let dest = file.path().to_path_buf();
    let bytes = copy_to_path(&source, &dest).await?;
    log::info!(
        "[artifact_commands] save_artifact_via_dialog bytes={bytes} dest={}",
        dest.display()
    );
    Ok(Some(dest.display().to_string()))
}

/// Validate a renderer-supplied source path: must be a non-empty,
/// absolute path that exists on disk AND resolve inside the OpenHuman
/// data directory's `artifacts/` tree. The path always originates from
/// the core `ai_get_artifact` RPC's `absolute_path`, but the command is
/// reachable by the renderer directly, so we re-validate the trust
/// boundary here — without the artifacts-root check a compromised
/// renderer could copy any readable local file out through the Save-As
/// dialog under an artifact-looking name (Codex P2).
fn validate_source(source_path: &str) -> Result<PathBuf, String> {
    if source_path.trim().is_empty() {
        return Err("source_path must not be empty".to_string());
    }
    let source = PathBuf::from(source_path);
    if !source.is_absolute() {
        return Err(format!(
            "source_path must be absolute (came from ai_get_artifact): {source_path:?}"
        ));
    }
    if !source.is_file() {
        return Err(format!(
            "artifact source not present on disk: {source_path}"
        ));
    }
    let root = crate::cef_profile::default_root_openhuman_dir()?;
    assert_artifact_source(&source, &root)?;
    Ok(source)
}

/// Confirm `source` resolves inside `root` (the OpenHuman data dir) and
/// carries an `artifacts` path component — i.e. it is a workspace
/// artifact, not an arbitrary local file. Canonicalizes both sides so
/// symlink trickery can't escape the root. Isolated for unit testing
/// without touching the real home directory.
fn assert_artifact_source(source: &Path, root: &Path) -> Result<(), String> {
    let canon_source = source
        .canonicalize()
        .map_err(|e| format!("cannot resolve source path: {e}"))?;
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !canon_source.starts_with(&canon_root) {
        return Err("source must be inside the OpenHuman data directory".to_string());
    }
    if !canon_source
        .components()
        .any(|c| c.as_os_str() == "artifacts")
    {
        return Err("source must be a workspace artifact file".to_string());
    }
    Ok(())
}

/// Copy `source` to `dest`, returning the byte count. Shared by the
/// Save-As dialog flow; isolated so it is unit-testable without driving
/// a real OS dialog.
async fn copy_to_path(source: &Path, dest: &Path) -> Result<u64, String> {
    tokio::fs::copy(source, dest)
        .await
        .map_err(|e| format!("failed to copy artifact to {:?}: {e}", dest))
}

/// Maximum number of `(N)` suffixes we'll append when picking a
/// non-colliding filename. After 1000 we give up and append a UUID
/// suffix instead so the download never silently overwrites.
const MAX_COLLISION_SUFFIX: u32 = 1000;

#[tauri::command]
pub async fn download_artifact_to_downloads(
    source_path: String,
    filename: String,
) -> Result<String, String> {
    let source = validate_source(&source_path)?;
    if filename.trim().is_empty() {
        return Err("filename must not be empty".to_string());
    }
    let sanitized = sanitize_filename(&filename)?;

    let downloads = directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
        .ok_or_else(|| "OS Downloads directory not resolvable".to_string())?;
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|e| format!("failed to ensure Downloads dir {:?}: {e}", downloads))?;

    let dest = pick_unique_path(&downloads, &sanitized);
    let bytes = copy_to_path(&source, &dest).await?;

    log::info!(
        "[artifact_commands] download_artifact_to_downloads bytes={bytes} dest={}",
        dest.display()
    );
    Ok(dest.display().to_string())
}

/// Strip path-traversal characters from a filename hint. The
/// renderer is expected to pass something like `"My Deck.pptx"`;
/// reject anything that contains a separator or null byte so a
/// malicious `ai_get_artifact` response can never escape the chosen dir.
fn sanitize_filename(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("filename must not be empty after trim".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(format!(
            "filename must not contain path separators: {trimmed:?}"
        ));
    }
    if trimmed.contains('\0') {
        return Err(format!("filename must not contain NUL bytes: {trimmed:?}"));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(format!("filename must not be '.' or '..': {trimmed:?}"));
    }
    Ok(trimmed.to_string())
}

/// Pick a destination path under `dir` that does not exist yet.
/// Inserts ` (N)` between the stem and the extension. Falls back to
/// a UUID suffix after [`MAX_COLLISION_SUFFIX`] tries.
fn pick_unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_stem_ext(filename);
    for n in 1..=MAX_COLLISION_SUFFIX {
        let nth = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dir.join(&nth);
        if !path.exists() {
            return path;
        }
    }
    // 1000 collisions is implausible in practice; if we hit it, fall
    // back to a monotonic nanosecond suffix so the copy still succeeds
    // without overwriting anything. Reaches for the OS clock instead of
    // pulling in `uuid` as a Tauri-shell dep just for this corner.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let with_uniq = if ext.is_empty() {
        format!("{stem}-{nanos}")
    } else {
        format!("{stem}-{nanos}.{ext}")
    };
    dir.join(with_uniq)
}

fn split_stem_ext(filename: &str) -> (String, String) {
    if let Some(idx) = filename.rfind('.') {
        // Reject leading-dot files (`.hidden`) — treat as having no extension.
        if idx > 0 && idx < filename.len() - 1 {
            return (filename[..idx].to_string(), filename[idx + 1..].to_string());
        }
    }
    (filename.to_string(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_path_separators() {
        assert!(sanitize_filename("../etc/passwd").is_err());
        assert!(sanitize_filename("a\\b.pptx").is_err());
        assert!(sanitize_filename("a/b.pptx").is_err());
        assert!(sanitize_filename("").is_err());
        assert!(sanitize_filename(".").is_err());
        assert!(sanitize_filename("..").is_err());
        assert!(sanitize_filename("ok.pptx\0").is_err());
    }

    #[test]
    fn sanitize_accepts_plain_names() {
        assert_eq!(
            sanitize_filename("Quarterly Update.pptx").unwrap(),
            "Quarterly Update.pptx"
        );
        assert_eq!(sanitize_filename("  trim me  ").unwrap(), "trim me");
    }

    #[test]
    fn validate_source_rejects_relative_and_empty() {
        assert!(validate_source("").is_err());
        assert!(validate_source("relative/path.pptx").is_err());
        assert!(validate_source("/definitely/not/here.pptx").is_err());
    }

    #[test]
    fn assert_artifact_source_accepts_file_under_artifacts_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let art = root.join("users/u1/workspace/artifacts/a-1");
        std::fs::create_dir_all(&art).unwrap();
        let file = art.join("deck.pptx");
        std::fs::write(&file, b"x").unwrap();
        assert!(assert_artifact_source(&file, root).is_ok());
    }

    #[test]
    fn assert_artifact_source_rejects_file_without_artifacts_component() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let other = root.join("users/u1/secrets");
        std::fs::create_dir_all(&other).unwrap();
        let file = other.join("token.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(assert_artifact_source(&file, root).is_err());
    }

    #[test]
    fn assert_artifact_source_rejects_file_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        // Even with an `artifacts` segment, a path outside the root is denied.
        let art = outside_dir.path().join("artifacts");
        std::fs::create_dir_all(&art).unwrap();
        let file = art.join("evil.pptx");
        std::fs::write(&file, b"x").unwrap();
        assert!(assert_artifact_source(&file, root_dir.path()).is_err());
    }

    #[tokio::test]
    async fn copy_to_path_copies_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.pptx");
        let dst = temp.path().join("dst.pptx");
        std::fs::write(&src, b"deck-bytes").unwrap();
        let n = copy_to_path(&src, &dst).await.unwrap();
        assert_eq!(n, b"deck-bytes".len() as u64);
        assert_eq!(std::fs::read(&dst).unwrap(), b"deck-bytes");
    }

    #[tokio::test]
    async fn save_via_dialog_rejects_bad_source() {
        // Validation runs before any dialog is shown, so these resolve
        // without user interaction.
        assert!(
            save_artifact_via_dialog(String::new(), "x.pptx".to_string())
                .await
                .is_err()
        );
        assert!(
            save_artifact_via_dialog("relative".to_string(), "x.pptx".to_string())
                .await
                .is_err()
        );
    }

    #[test]
    fn split_stem_ext_pairs() {
        assert_eq!(
            split_stem_ext("file.pptx"),
            ("file".to_string(), "pptx".to_string())
        );
        assert_eq!(
            split_stem_ext("noext"),
            ("noext".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext(".hidden"),
            (".hidden".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext("trailing."),
            ("trailing.".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext("a.b.c"),
            ("a.b".to_string(), "c".to_string())
        );
    }

    #[test]
    fn pick_unique_inserts_collision_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = pick_unique_path(dir, "deck.pptx");
        assert_eq!(first, dir.join("deck.pptx"));

        std::fs::write(&first, b"").unwrap();
        let second = pick_unique_path(dir, "deck.pptx");
        assert_eq!(second, dir.join("deck (1).pptx"));

        std::fs::write(&second, b"").unwrap();
        let third = pick_unique_path(dir, "deck.pptx");
        assert_eq!(third, dir.join("deck (2).pptx"));
    }

    #[test]
    fn pick_unique_handles_no_extension() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = pick_unique_path(dir, "noext");
        assert_eq!(first, dir.join("noext"));
        std::fs::write(&first, b"").unwrap();
        let second = pick_unique_path(dir, "noext");
        assert_eq!(second, dir.join("noext (1)"));
    }

    #[tokio::test]
    async fn download_rejects_invalid_inputs() {
        assert!(
            download_artifact_to_downloads(String::new(), "x.pptx".to_string())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("/tmp/x".to_string(), String::new())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("relative".to_string(), "x.pptx".to_string())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("/nope".to_string(), "../escape.pptx".to_string())
                .await
                .is_err()
        );
    }
}
