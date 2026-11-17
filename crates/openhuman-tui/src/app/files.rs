//! Bounded workspace file discovery.
use super::*;

pub(super) async fn open_file_picker(ui: &mut UiState) {
    let root = ui.action_dir.clone();
    let query = ui
        .composer
        .file_query()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let result = tokio::task::spawn_blocking(move || collect_files(&root, &query, 500)).await;
    let mut overlay = Overlay::new(OverlayKind::Files, "Workspace files");
    match result {
        Ok(Ok(files)) => {
            overlay.rows = files
                .into_iter()
                .map(|path| text_row(&path, &path, "Enter inserts this path"))
                .collect();
            overlay.status = format!(
                "{} match(es) · type to filter · Esc closes",
                overlay.rows.len()
            );
        }
        Ok(Err(error)) => overlay.status = error,
        Err(error) => overlay.status = error.to_string(),
    }
    ui.overlay = Some(overlay);
}

pub(super) fn collect_files(root: &str, query: &str, limit: usize) -> Result<Vec<String>, String> {
    const MAX_VISITED: usize = 25_000;
    const MAX_DEPTH: usize = 8;
    if root.is_empty() {
        return Err("Action directory is unavailable.".into());
    }
    let root_path = std::path::Path::new(root);
    let mut pending = vec![(root_path.to_path_buf(), 0usize)];
    let mut files = Vec::new();
    let mut visited = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        if visited >= MAX_VISITED {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == root_path => {
                return Err(format!("{}: {error}", dir.display()));
            }
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if depth < MAX_DEPTH
                    && !matches!(
                        name.as_ref(),
                        ".git" | "target" | "node_modules" | "worktrees"
                    )
                {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            let relative = path
                .strip_prefix(root_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if query.is_empty() || relative.to_ascii_lowercase().contains(query) {
                files.push(relative);
                if files.len() >= limit {
                    files.sort();
                    return Ok(files);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}
