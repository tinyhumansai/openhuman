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

/// Maximum length of a sanitized artifact filename stem. Keeps the
/// rendered filename short enough to round-trip on every filesystem
/// (Windows MAX_PATH, ext4 NAME_MAX) without truncating the
/// `.extension` suffix or the UUID-named parent directory.
const MAX_SANITIZED_FILENAME_LEN: usize = 80;

/// Convert a human-readable title into a filesystem-safe filename
/// stem. Strips path-traversal characters, collapses whitespace to
/// single dashes, lowercases, and caps the length. Falls back to
/// `"artifact"` when the resulting stem is empty (e.g. title was
/// `"///"` or only emoji that survive ASCII-only sanitisation).
fn sanitize_filename_stem(title: &str) -> String {
    let mut out = String::with_capacity(title.len().min(MAX_SANITIZED_FILENAME_LEN));
    let mut prev_dash = false;
    for ch in title.chars() {
        let mapped = match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch.to_ascii_lowercase(),
            ' ' | '\t' | '\n' | '\r' | '.' | '/' | '\\' | ':' => '-',
            _ => continue,
        };
        if mapped == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
        } else {
            prev_dash = false;
        }
        out.push(mapped);
        if out.chars().count() >= MAX_SANITIZED_FILENAME_LEN {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed
    }
}

/// Allocate a fresh artifact directory and persist a pending
/// [`ArtifactMeta`] record. Returns the metadata plus the absolute
/// path where the producer should write the artifact bytes.
///
/// On success the caller MUST follow up with [`finalize_artifact`]
/// once the bytes are on disk (or [`fail_artifact`] if generation
/// failed) so the status flips off `Pending`. Leaving a record in
/// `Pending` is harmless — the list RPC will still surface it — but
/// downstream consumers (UI, download endpoints) treat `Pending` as
/// "not yet ready", so a stuck record means a stuck spinner.
///
/// `extension` is the file extension WITHOUT the leading dot
/// (e.g. `"pptx"`, `"pdf"`). Used to build the rendered filename
/// under the artifact directory.
pub async fn create_artifact(
    workspace_dir: &Path,
    kind: super::types::ArtifactKind,
    title: &str,
    extension: &str,
) -> Result<(ArtifactMeta, PathBuf), String> {
    let trimmed_title = title.trim();
    if trimmed_title.is_empty() {
        return Err("[artifacts] create_artifact: title must not be empty".to_string());
    }
    let trimmed_ext = extension.trim();
    if trimmed_ext.is_empty() {
        return Err("[artifacts] create_artifact: extension must not be empty".to_string());
    }
    if trimmed_ext.contains('/') || trimmed_ext.contains('\\') || trimmed_ext.contains('.') {
        return Err(format!(
            "[artifacts] create_artifact: extension must not contain '/', '\\', or '.': {trimmed_ext:?}"
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{}.{trimmed_ext}", sanitize_filename_stem(trimmed_title));
    let relative_path = format!("{id}/{filename}");

    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(&id);
    assert_within_root(&root, &artifact_dir)?;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|e| {
            format!(
                "[artifacts] create_artifact: failed to mkdir {:?}: {e}",
                artifact_dir
            )
        })?;
    let absolute_path = artifact_dir.join(&filename);

    let meta = ArtifactMeta {
        id: id.clone(),
        kind,
        title: trimmed_title.to_string(),
        path: relative_path,
        size_bytes: 0,
        status: ArtifactStatus::Pending,
        created_at: chrono::Utc::now(),
        error: None,
    };
    save_artifact_meta(workspace_dir, &meta).await?;

    log::debug!(
        "[artifacts] create_artifact: id={id} kind={} path={:?}",
        meta.kind.as_str(),
        absolute_path
    );
    Ok((meta, absolute_path))
}

/// Flip a pending artifact to [`ArtifactStatus::Ready`] and persist
/// the final size. Idempotent on already-ready artifacts (no-op + log).
/// Returns the updated metadata.
///
/// On a real transition (Pending → Ready), publishes
/// [`DomainEvent::ArtifactReady`] on the global bus so the web
/// channel can surface a download card to the originating thread.
/// When the calling task carries no
/// [`ApprovalChatContext`](crate::openhuman::approval::ApprovalChatContext)
/// (CLI / cron / sub-agent paths), the event is still published but
/// `thread_id` / `client_id` are `None` so the socket bridge silently
/// drops it. Idempotent calls (already-Ready) skip the publish so we
/// don't flap the UI.
pub async fn finalize_artifact(
    workspace_dir: &Path,
    artifact_id: &str,
    size_bytes: u64,
) -> Result<ArtifactMeta, String> {
    let mut meta = get_artifact(workspace_dir, artifact_id).await?;
    if matches!(meta.status, ArtifactStatus::Ready) && meta.size_bytes == size_bytes {
        log::debug!("[artifacts] finalize_artifact: id={artifact_id} already Ready, no-op");
        return Ok(meta);
    }
    meta.status = ArtifactStatus::Ready;
    meta.size_bytes = size_bytes;
    meta.error = None;
    save_artifact_meta(workspace_dir, &meta).await?;
    log::debug!("[artifacts] finalize_artifact: id={artifact_id} -> Ready size={size_bytes}");

    let (thread_id, client_id) = current_chat_context();
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::ArtifactReady {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        path: meta.path.clone(),
        size_bytes: meta.size_bytes,
        thread_id,
        client_id,
    });
    Ok(meta)
}

/// Flip an artifact to [`ArtifactStatus::Failed`] and persist a
/// failure reason. The producer should call this when generation
/// fails so the UI / RPC consumer can surface a useful message
/// instead of an indefinite spinner. Returns the updated metadata.
///
/// Publishes [`DomainEvent::ArtifactFailed`] so the chat surface
/// flips the in-flight card to a retry-hint state. Same chat-context
/// rules as [`finalize_artifact`].
pub async fn fail_artifact(
    workspace_dir: &Path,
    artifact_id: &str,
    reason: &str,
) -> Result<ArtifactMeta, String> {
    let mut meta = get_artifact(workspace_dir, artifact_id).await?;
    meta.status = ArtifactStatus::Failed;
    meta.error = Some(reason.to_string());
    save_artifact_meta(workspace_dir, &meta).await?;
    // Log only the size of the reason — it can carry provider stderr
    // / user-derived content, which we don't want flushed verbatim
    // into structured logs. The full payload is still persisted on
    // `meta.error` for the UI surface and the chat event below.
    log::warn!(
        "[artifacts] fail_artifact: id={artifact_id} -> Failed reason_len={}",
        reason.len()
    );

    let (thread_id, client_id) = current_chat_context();
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::ArtifactFailed {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        error: reason.to_string(),
        thread_id,
        client_id,
    });
    Ok(meta)
}

/// Read the active [`ApprovalChatContext`] task-local (set by
/// `channels::providers::web` around each chat turn) and return its
/// thread + client ids. Returns `(None, None)` for non-chat callers
/// (CLI, cron, sub-agent runners) so artifact emit hooks degrade
/// gracefully — the event is still published but the web subscriber
/// drops it for lack of a routing target.
fn current_chat_context() -> (Option<String>, Option<String>) {
    crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|ctx| (Some(ctx.thread_id.clone()), Some(ctx.client_id.clone())))
        .unwrap_or((None, None))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
