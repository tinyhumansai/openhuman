use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const DEFAULT_PREVIEW_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceTextPreview {
    pub path: String,
    pub absolute_path: String,
    pub contents: String,
    pub truncated: bool,
    pub size_bytes: u64,
}

#[tauri::command]
pub async fn open_workspace_path(path: String) -> Result<(), String> {
    let workspace = active_workspace_root().await?;
    let target = resolve_workspace_path(&workspace, &path)?;
    let workspace_path = workspace_path_label(&workspace, &target);
    tauri_plugin_opener::open_path(&target, None::<&str>).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to open workspace path {workspace_path}: {err}"),
            format!("failed to open workspace path {}: {err}", target.display()),
        )
    })
}

#[tauri::command]
pub async fn reveal_workspace_path(path: String) -> Result<(), String> {
    let workspace = active_workspace_root().await?;
    let target = resolve_workspace_path(&workspace, &path)?;
    let workspace_path = workspace_path_label(&workspace, &target);
    tauri_plugin_opener::reveal_item_in_dir(&target).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to reveal workspace path {workspace_path}: {err}"),
            format!(
                "failed to reveal workspace path {}: {err}",
                target.display()
            ),
        )
    })
}

#[tauri::command]
pub async fn preview_workspace_text(path: String) -> Result<WorkspaceTextPreview, String> {
    let workspace = active_workspace_root().await?;
    preview_workspace_text_from_root(&workspace, &path, DEFAULT_PREVIEW_MAX_BYTES)
}

async fn active_workspace_root() -> Result<PathBuf, String> {
    let config = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .map_err(|err| workspace_path_error(format!("failed to load OpenHuman config: {err}")))?;
    fs::create_dir_all(&config.workspace_dir).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to create workspace directory: {err}"),
            format!(
                "failed to create workspace directory {}: {err}",
                config.workspace_dir.display()
            ),
        )
    })?;
    Ok(config.workspace_dir)
}

fn workspace_path_error(message: impl Into<String>) -> String {
    let message = message.into();
    log::warn!("[workspace-paths] {message}");
    message
}

fn workspace_path_error_with_debug(
    message: impl Into<String>,
    debug_message: impl Into<String>,
) -> String {
    let message = message.into();
    log::warn!("[workspace-paths] {message}");
    log::debug!("[workspace-paths] {}", debug_message.into());
    message
}

fn workspace_path_label(workspace_root: &Path, target: &Path) -> String {
    let relative = fs::canonicalize(workspace_root)
        .ok()
        .and_then(|root| target.strip_prefix(root).ok().map(Path::to_path_buf));

    relative
        .as_deref()
        .map(path_label)
        .or_else(|| {
            target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "<redacted>".to_string())
}

fn path_label(path: &Path) -> String {
    let label = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");

    if label.is_empty() {
        ".".to_string()
    } else {
        label
    }
}

fn normalize_workspace_relative_path(path: &str) -> Result<(PathBuf, String), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(workspace_path_error("workspace path must not be empty"));
    }
    if trimmed.bytes().any(|byte| byte == 0) {
        return Err(workspace_path_error(
            "workspace path must not contain NUL bytes",
        ));
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/')
        || has_windows_drive_prefix(&normalized)
        || has_uri_scheme_prefix(&normalized)
    {
        return Err(workspace_path_error("workspace path must be relative"));
    }

    let mut relative = PathBuf::new();
    let mut clean_parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(workspace_path_error(
                "workspace path must stay inside the workspace",
            ));
        }
        relative.push(part);
        clean_parts.push(part);
    }

    if clean_parts.is_empty() {
        return Err(workspace_path_error(
            "workspace path must point to a file or directory",
        ));
    }

    Ok((relative, clean_parts.join("/")))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn has_uri_scheme_prefix(path: &str) -> bool {
    let Some((scheme, _)) = path.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

pub(crate) fn resolve_workspace_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, String> {
    let (relative, normalized_path) = normalize_workspace_relative_path(requested_path)?;
    let root = fs::canonicalize(workspace_root).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to canonicalize workspace directory: {err}"),
            format!(
                "failed to canonicalize workspace directory {}: {err}",
                workspace_root.display()
            ),
        )
    })?;
    let target = root.join(relative);
    let target = fs::canonicalize(&target).map_err(|err| {
        workspace_path_error(format!(
            "workspace path does not exist {normalized_path}: {err}"
        ))
    })?;

    if !target.starts_with(&root) {
        return Err(workspace_path_error_with_debug(
            format!("workspace path must stay inside the workspace: {normalized_path}"),
            format!(
                "workspace path must stay inside the workspace: {} -> {}",
                normalized_path,
                target.display()
            ),
        ));
    }

    log::debug!(
        "[workspace-paths] resolved workspace path: {} -> {}",
        normalized_path,
        target.display()
    );
    Ok(target)
}

pub(crate) fn preview_workspace_text_from_root(
    workspace_root: &Path,
    requested_path: &str,
    max_bytes: usize,
) -> Result<WorkspaceTextPreview, String> {
    let (_, normalized_path) = normalize_workspace_relative_path(requested_path)?;
    let target = resolve_workspace_path(workspace_root, &normalized_path)?;
    let metadata = fs::metadata(&target).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to read metadata for {normalized_path}: {err}"),
            format!("failed to read metadata for {}: {err}", target.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(workspace_path_error(format!(
            "workspace preview target must be a file: {normalized_path}"
        )));
    }

    let mut file = fs::File::open(&target).map_err(|err| {
        workspace_path_error_with_debug(
            format!("failed to open workspace file {normalized_path}: {err}"),
            format!("failed to open workspace file {}: {err}", target.display()),
        )
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(4) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| {
            workspace_path_error_with_debug(
                format!("failed to read workspace file {normalized_path}: {err}"),
                format!("failed to read workspace file {}: {err}", target.display()),
            )
        })?;

    let truncated = metadata.len() > max_bytes as u64;
    let preview_len = bytes.len().min(max_bytes);
    let contents = utf8_preview(&bytes[..preview_len], truncated).map_err(|err| {
        workspace_path_error_with_debug(
            format!("{err}: {normalized_path}"),
            format!("{err}: {}", target.display()),
        )
    })?;

    log::debug!(
        "[workspace-paths] previewed workspace text: {} bytes={} truncated={}",
        normalized_path,
        metadata.len(),
        truncated
    );

    Ok(WorkspaceTextPreview {
        path: normalized_path,
        absolute_path: target.display().to_string(),
        contents,
        truncated,
        size_bytes: metadata.len(),
    })
}

fn utf8_preview(bytes: &[u8], truncated: bool) -> Result<String, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(err) if truncated && err.error_len().is_none() => {
            Ok(String::from_utf8_lossy(&bytes[..err.valid_up_to()]).into_owned())
        }
        Err(_) => Err("workspace preview target is not valid UTF-8 text".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolve_workspace_path_accepts_existing_relative_file_inside_workspace() {
        let workspace = tempdir().unwrap();
        let docs = workspace.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        let file = docs.join("note.md");
        fs::write(&file, "hello").unwrap();

        let resolved = resolve_workspace_path(workspace.path(), "docs/note.md").unwrap();

        assert_eq!(resolved, file.canonicalize().unwrap());
    }

    #[test]
    fn resolve_workspace_path_rejects_parent_directory_escape() {
        let workspace = tempdir().unwrap();

        let err = resolve_workspace_path(workspace.path(), "../secret.txt").unwrap_err();

        assert!(err.contains("workspace"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_workspace_path_rejects_absolute_paths() {
        let workspace = tempdir().unwrap();

        let err = resolve_workspace_path(workspace.path(), "/etc/passwd").unwrap_err();

        assert!(err.contains("relative"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_workspace_path_rejects_uri_scheme_prefix() {
        let workspace = tempdir().unwrap();

        let err = resolve_workspace_path(workspace.path(), "file://etc/passwd").unwrap_err();

        assert!(err.contains("relative"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_workspace_path_accepts_colons_after_first_segment() {
        let workspace = tempdir().unwrap();
        let docs = workspace.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        let file = docs.join("2026:05.md");
        fs::write(&file, "dated").unwrap();

        let resolved = resolve_workspace_path(workspace.path(), "docs/2026:05.md").unwrap();

        assert_eq!(resolved, file.canonicalize().unwrap());
    }

    #[test]
    fn resolve_workspace_path_errors_do_not_expose_workspace_root() {
        let workspace = tempdir().unwrap();

        let err = resolve_workspace_path(workspace.path(), "docs/missing.md").unwrap_err();

        assert!(err.contains("docs/missing.md"), "unexpected error: {err}");
        assert!(
            !err.contains(&workspace.path().display().to_string()),
            "error leaked workspace root: {err}"
        );
    }

    #[test]
    fn preview_workspace_text_from_root_reads_utf8_text() {
        let workspace = tempdir().unwrap();
        fs::write(workspace.path().join("readme.md"), "# Hello").unwrap();

        let preview =
            preview_workspace_text_from_root(workspace.path(), "readme.md", 1024).unwrap();

        assert_eq!(preview.path, "readme.md");
        assert_eq!(preview.contents, "# Hello");
        assert!(!preview.truncated);
        assert_eq!(preview.size_bytes, 7);
    }

    #[test]
    fn preview_workspace_text_from_root_truncates_large_text() {
        let workspace = tempdir().unwrap();
        fs::write(workspace.path().join("large.md"), "0123456789").unwrap();

        let preview = preview_workspace_text_from_root(workspace.path(), "large.md", 4).unwrap();

        assert_eq!(preview.contents, "0123");
        assert!(preview.truncated);
        assert_eq!(preview.size_bytes, 10);
    }

    #[test]
    fn preview_workspace_text_from_root_errors_do_not_expose_workspace_root() {
        let workspace = tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("docs")).unwrap();

        let err = preview_workspace_text_from_root(workspace.path(), "docs", 1024).unwrap_err();

        assert!(err.contains("docs"), "unexpected error: {err}");
        assert!(
            !err.contains(&workspace.path().display().to_string()),
            "error leaked workspace root: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();
        symlink(&outside_file, workspace.path().join("secret-link")).unwrap();

        let err = resolve_workspace_path(workspace.path(), "secret-link").unwrap_err();

        assert!(err.contains("workspace"), "unexpected error: {err}");
    }
}
