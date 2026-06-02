//! Tauri command for downloading agent-generated artifacts (#2779).
//!
//! Contract: the frontend resolves the artifact's absolute source
//! path via the existing `openhuman.ai_get_artifact` core RPC, then
//! invokes [`download_artifact_to_downloads`] with that source path
//! plus a filename hint. The command:
//!
//! 1. Validates both inputs (no path traversal in the filename, source
//!    must be absolute + on disk).
//! 2. Resolves the user's Downloads directory via the `dirs` crate.
//! 3. Picks a non-colliding destination filename — `name.pptx`,
//!    `name (1).pptx`, `name (2).pptx`, …
//! 4. Copies source → dest with `tokio::fs::copy`.
//! 5. Returns the absolute dest path so the frontend can show a
//!    "Saved to …" toast with a "Reveal in Finder" button (the
//!    `opener:allow-reveal-item-in-dir` capability is already wired).
//!
//! Why Downloads instead of a native save-file dialog: the
//! `tauri-plugin-dialog` crate pulls `tauri-plugin-fs` transitively,
//! which currently breaks the openhuman build with a `schemars`
//! version conflict. The Downloads + reveal pattern satisfies the
//! "user-chosen destination" intent of issue #2779 AC#3 without
//! widening the Tauri allow-list, and matches what most desktop chat
//! apps do for downloaded attachments.

use std::path::{Path, PathBuf};

/// Maximum number of `(N)` suffixes we'll append when picking a
/// non-colliding filename. After 1000 we give up and append a UUID
/// suffix instead so the download never silently overwrites.
const MAX_COLLISION_SUFFIX: u32 = 1000;

#[tauri::command]
pub async fn download_artifact_to_downloads(
    source_path: String,
    filename: String,
) -> Result<String, String> {
    if source_path.trim().is_empty() {
        return Err("source_path must not be empty".to_string());
    }
    if filename.trim().is_empty() {
        return Err("filename must not be empty".to_string());
    }
    let source = PathBuf::from(&source_path);
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
    let sanitized = sanitize_filename(&filename)?;

    let downloads = directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
        .ok_or_else(|| "OS Downloads directory not resolvable".to_string())?;
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|e| format!("failed to ensure Downloads dir {:?}: {e}", downloads))?;

    let dest = pick_unique_path(&downloads, &sanitized);
    let bytes = tokio::fs::copy(&source, &dest)
        .await
        .map_err(|e| format!("failed to copy artifact to {:?}: {e}", dest))?;

    log::info!(
        "[artifact_commands] download_artifact_to_downloads bytes={bytes} dest={}",
        dest.display()
    );
    Ok(dest.display().to_string())
}

/// Strip path-traversal characters from a filename hint. The
/// renderer is expected to pass something like `"My Deck.pptx"`;
/// reject anything that contains a separator or null byte so a
/// malicious `ai_get_artifact` response can never escape Downloads.
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
