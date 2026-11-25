//! Tauri commands for exporting agent-generated artifacts (#2779).
//!
//! Two export paths, both fed by an artifact id:
//!
//! - [`save_artifact_as`] asks the user where to save it (the OS save dialog,
//!   through the `rfd` the shell links for the directory chooser) and copies
//!   the file there. The preview panel's Download uses it, through the
//!   frontend's `saveArtifactViaDialog`.
//! - [`download_artifact_to_downloads`] copies the file into the user's
//!   Downloads directory with a non-colliding name and returns the dest path,
//!   so the UI can offer "Reveal in Finder". Cross-platform.
//!
//! The renderer names an artifact, never a path (#5505). Artifact files now
//! live in a visible, user-configurable folder rather than under the data
//! dir, so the shell cannot check a renderer-supplied path against a fixed
//! root; instead it asks the core, whose `resolve_ready_file` applies the
//! artifact escape guard and refuses records that are not `Ready`. A
//! compromised renderer can therefore only copy out files the artifact store
//! already vouches for. The filename hint is still sanitized so nothing is
//! written outside the Downloads directory.

use std::path::{Path, PathBuf};

/// Resolve an artifact id to the file the core's artifact store vouches for,
/// in the given workspace. Isolated from config loading for unit testing.
async fn resolve_source(
    workspace_dir: &Path,
    roots: &openhuman_rpc::embed::artifacts::FileRoots,
    artifact_id: &str,
) -> Result<PathBuf, String> {
    let artifact_id = artifact_id.trim();
    if artifact_id.is_empty() {
        return Err("artifact_id must not be empty".to_string());
    }
    openhuman_rpc::embed::artifacts::resolve_ready_file(workspace_dir, roots, artifact_id).await
}

/// Copy `source` to `dest`, returning the byte count. Isolated so it is
/// unit-testable without touching the real Downloads directory.
async fn copy_to_path(source: &Path, dest: &Path) -> Result<u64, String> {
    tokio::fs::copy(source, dest)
        .await
        .map_err(|e| format!("failed to copy artifact to {:?}: {e}", dest))
}

/// Ask the user where to save an artifact (the OS save dialog, prefilled with
/// `filename`) and copy the artifact's file there.
///
/// Returns `Ok(Some(path))` once copied, `Ok(None)` when the user cancels.
/// The file copied is the one the core's artifact store vouches for; for a
/// versioned artifact that is its latest version.
#[tauri::command]
pub async fn save_artifact_as(
    artifact_id: String,
    filename: String,
) -> Result<Option<String>, String> {
    let config = openhuman_rpc::embed::config::load_host_config().await?;
    let roots = openhuman_rpc::embed::artifacts::FileRoots::from_config(&config);
    save_to_chosen_path(
        &config.workspace_dir,
        &roots,
        &artifact_id,
        &filename,
        |name| async move {
            // macOS: NSSavePanel. Windows: IFileSaveDialog. Linux: the GTK chooser.
            rfd::AsyncFileDialog::new()
                .set_file_name(&name)
                .save_file()
                .await
                .map(|handle| handle.path().to_path_buf())
        },
    )
    .await
}

/// [`save_artifact_as`] with the destination chooser passed in, so the checks
/// run without a dialog: the id must resolve to a file the artifact store
/// vouches for and the filename must be a plain name, both before `choose` is
/// asked. `choose` gets the sanitized name and returns `None` on cancel.
async fn save_to_chosen_path<F, Fut>(
    workspace_dir: &Path,
    roots: &openhuman_rpc::embed::artifacts::FileRoots,
    artifact_id: &str,
    filename: &str,
    choose: F,
) -> Result<Option<String>, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Option<PathBuf>>,
{
    let source = resolve_source(workspace_dir, roots, artifact_id).await?;
    let sanitized = sanitize_filename(filename)?;
    let Some(dest) = choose(sanitized).await else {
        log::debug!("[artifact_commands] save_artifact_as cancelled by user");
        return Ok(None);
    };
    let bytes = copy_to_path(&source, &dest).await?;
    // The chosen path carries the user's login and folder names; it is not
    // logged (see `directory_picker`).
    log::info!("[artifact_commands] save_artifact_as bytes={bytes}");
    Ok(Some(dest.display().to_string()))
}

/// Maximum number of `(N)` suffixes we'll append when picking a
/// non-colliding filename. After 1000 we give up and append a UUID
/// suffix instead so the download never silently overwrites.
const MAX_COLLISION_SUFFIX: u32 = 1000;

#[tauri::command]
pub async fn download_artifact_to_downloads(
    artifact_id: String,
    filename: String,
) -> Result<String, String> {
    if artifact_id.trim().is_empty() {
        return Err("artifact_id must not be empty".to_string());
    }
    let config = openhuman_rpc::embed::config::load_host_config().await?;
    let roots = openhuman_rpc::embed::artifacts::FileRoots::from_config(&config);
    let source = resolve_source(&config.workspace_dir, &roots, &artifact_id).await?;
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

/// Turn a filename hint (the artifact title, e.g. `"Q3: plan.pptx"`) into a
/// single plain path component. Separators, NUL and `.`/`..` are rejected so
/// a malicious `ai_get_artifact` response can never escape the chosen dir.
/// Characters Windows treats specially are replaced rather than rejected —
/// `:` alone would otherwise make `C:evil.dll` drive-relative and
/// `a.txt:stream` an alternate data stream — and reserved device names
/// (`CON`, `NUL`, `COM1`, …) get a `_` prefix.
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
    let mapped: String = trimmed
        .chars()
        .map(|c| match c {
            ':' | '<' | '>' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces, so `evil.dll.` would land as
    // `evil.dll`.
    let mut name = mapped.trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() {
        return Err(format!("filename has no usable characters: {trimmed:?}"));
    }
    // Windows ignores spaces before the extension, so `CON .txt` is `CON`.
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    // Win32 also treats the superscript digits ¹ ² ³ as COM/LPT port digits.
    let port_digit = |rest: &str| {
        let mut chars = rest.chars();
        matches!(
            (chars.next(), chars.next()),
            (Some('1'..='9' | '¹' | '²' | '³'), None)
        )
    };
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"]
        .iter()
        .any(|p| stem.strip_prefix(p).is_some_and(port_digit));
    if reserved {
        name.insert(0, '_');
    }
    let mut components = Path::new(&name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        return Err(format!(
            "filename must be a single path component: {trimmed:?}"
        ));
    }
    Ok(name)
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
#[path = "artifact_commands_tests.rs"]
mod tests;
