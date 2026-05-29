use std::path::{Path, PathBuf};

use super::types::{ArtifactMeta, ArtifactStatus};

const ARTIFACTS_SUBDIR: &str = "artifacts";
const META_FILENAME: &str = "meta.json";

/// Returns the artifacts root directory, creating it if it doesn't exist.
///
/// The root lives at `<workspace_dir>/artifacts/`.
pub(crate) async fn artifacts_root(workspace_dir: &Path) -> Result<PathBuf, String> {
    let root = workspace_dir.join(ARTIFACTS_SUBDIR);
    log::debug!("[artifacts] artifacts_root: {:?}", root);
    tokio::fs::create_dir_all(&root).await.map_err(|e| {
        format!(
            "[artifacts] failed to create artifacts root {:?}: {e}",
            root
        )
    })?;
    Ok(root)
}

/// Validate that an artifact ID is safe to use as a filesystem path component.
///
/// Rejects empty strings, absolute paths, and path traversal patterns.
fn validate_artifact_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }
    if id == "." {
        return Err("[artifacts] artifact_id must not be '.'".to_string());
    }
    if id.contains('/') {
        return Err(format!(
            "[artifacts] artifact_id must not contain '/': {id:?}"
        ));
    }
    if id.contains('\\') {
        return Err(format!(
            "[artifacts] artifact_id must not contain '\\': {id:?}"
        ));
    }
    if id == ".." || id.starts_with("../") || id.starts_with("..\\") {
        return Err(format!(
            "[artifacts] artifact_id must not be a path traversal: {id:?}"
        ));
    }
    // Reject absolute paths (Unix /foo or Windows C:\foo / \\server\share)
    if id.starts_with('/') || id.starts_with('\\') {
        return Err(format!(
            "[artifacts] artifact_id must not be an absolute path: {id:?}"
        ));
    }
    // Reject Windows drive-letter paths like C:
    if id.len() >= 2 && id.as_bytes()[1] == b':' {
        return Err(format!(
            "[artifacts] artifact_id must not be an absolute path: {id:?}"
        ));
    }
    Ok(())
}

/// Confirm that `resolved` is under `root`, preventing path traversal escapes.
fn assert_within_root(root: &Path, resolved: &Path) -> Result<(), String> {
    if !resolved.starts_with(root) {
        return Err(format!(
            "[artifacts] path {:?} escapes artifacts root {:?}",
            resolved, root
        ));
    }
    Ok(())
}

/// Persist artifact metadata to `<workspace>/artifacts/<id>/meta.json`.
pub(crate) async fn save_artifact_meta(
    workspace_dir: &Path,
    meta: &ArtifactMeta,
) -> Result<(), String> {
    log::debug!("[artifacts] save_artifact_meta: id={}", meta.id);
    validate_artifact_id(&meta.id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(&meta.id);
    // Verify sandboxing before writing
    assert_within_root(&root, &artifact_dir)?;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|e| {
            format!(
                "[artifacts] failed to create artifact dir {:?}: {e}",
                artifact_dir
            )
        })?;
    let meta_path = artifact_dir.join(META_FILENAME);
    let json = serde_json::to_string_pretty(meta).map_err(|e| {
        format!(
            "[artifacts] failed to serialize meta for id={}: {e}",
            meta.id
        )
    })?;
    tokio::fs::write(&meta_path, json).await.map_err(|e| {
        format!(
            "[artifacts] failed to write meta.json for id={}: {e}",
            meta.id
        )
    })?;
    log::debug!("[artifacts] saved meta.json for id={}", meta.id);
    Ok(())
}

/// List artifacts in the workspace, sorted by `created_at` descending.
///
/// Corrupt or unreadable `meta.json` files are skipped with a `warn!` log.
/// Returns `(page, total)` where `page` is the requested slice and `total` is
/// the count before pagination.
pub(crate) async fn list_artifacts(
    workspace_dir: &Path,
    offset: usize,
    limit: usize,
) -> Result<(Vec<ArtifactMeta>, usize), String> {
    log::debug!(
        "[artifacts] list_artifacts: offset={offset} limit={limit} workspace={:?}",
        workspace_dir
    );
    let root = artifacts_root(workspace_dir).await?;

    let mut read_dir = match tokio::fs::read_dir(&root).await {
        Ok(rd) => rd,
        Err(e) => {
            return Err(format!(
                "[artifacts] failed to read artifacts dir {:?}: {e}",
                root
            ))
        }
    };

    let mut all: Vec<ArtifactMeta> = Vec::new();

    loop {
        let entry = match read_dir.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                log::warn!("[artifacts] error reading directory entry: {e}");
                continue;
            }
        };

        let entry_path = entry.path();
        // Only process directories
        match entry.file_type().await {
            Ok(ft) if ft.is_dir() => {}
            Ok(_) => continue,
            Err(e) => {
                log::warn!(
                    "[artifacts] failed to get file type for {:?}: {e}",
                    entry_path
                );
                continue;
            }
        }

        let meta_path = entry_path.join(META_FILENAME);
        let contents = match tokio::fs::read_to_string(&meta_path).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!(
                    "[artifacts] skipping {:?}: failed to read meta.json: {e}",
                    entry_path
                );
                continue;
            }
        };

        match serde_json::from_str::<ArtifactMeta>(&contents) {
            Ok(meta) => all.push(meta),
            Err(e) => {
                log::warn!(
                    "[artifacts] skipping {:?}: corrupt meta.json: {e}",
                    entry_path
                );
            }
        }
    }

    // Sort descending by created_at (newest first)
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let total = all.len();
    let page = all.into_iter().skip(offset).take(limit).collect::<Vec<_>>();

    log::debug!(
        "[artifacts] list_artifacts: total={total} returning {} items",
        page.len()
    );
    Ok((page, total))
}

/// Retrieve a single artifact by ID.
pub(crate) async fn get_artifact(
    workspace_dir: &Path,
    artifact_id: &str,
) -> Result<ArtifactMeta, String> {
    log::debug!("[artifacts] get_artifact: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(artifact_id);
    assert_within_root(&root, &artifact_dir)?;
    let meta_path = artifact_dir.join(META_FILENAME);
    let contents = tokio::fs::read_to_string(&meta_path).await.map_err(|e| {
        format!("[artifacts] artifact not found or unreadable id={artifact_id}: {e}")
    })?;
    let meta: ArtifactMeta = serde_json::from_str(&contents)
        .map_err(|e| format!("[artifacts] corrupt meta.json for id={artifact_id}: {e}"))?;
    log::debug!("[artifacts] get_artifact: found id={artifact_id}");
    Ok(meta)
}

/// Delete an artifact directory and all its contents.
pub(crate) async fn delete_artifact(workspace_dir: &Path, artifact_id: &str) -> Result<(), String> {
    log::debug!("[artifacts] delete_artifact: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(artifact_id);
    assert_within_root(&root, &artifact_dir)?;
    tokio::fs::remove_dir_all(&artifact_dir)
        .await
        .map_err(|e| format!("[artifacts] failed to delete artifact id={artifact_id}: {e}"))?;
    log::debug!("[artifacts] delete_artifact: deleted id={artifact_id}");
    Ok(())
}

// Mark a status as unused — referenced only in tests via the store
#[allow(dead_code)]
fn _assert_status_used(_: ArtifactStatus) {}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
