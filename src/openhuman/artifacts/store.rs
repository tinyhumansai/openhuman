use std::path::{Path, PathBuf};

use super::types::{ArtifactMeta, ArtifactStatus};

const ARTIFACTS_SUBDIR: &str = "artifacts";
const META_FILENAME: &str = "meta.json";
/// Sidecar file holding the verbatim producer-tool arguments that
/// generated the artifact, persisted next to `meta.json` so a failed
/// card's Retry button can re-dispatch the exact same generation
/// deterministically without round-tripping the args back through the
/// LLM (#3162). Written by the producing tool right after
/// [`create_artifact`]; read by `ops::ai_regenerate`.
const ARGS_FILENAME: &str = "args.json";

tokio::task_local! {
    /// When set (by `ops::ai_regenerate`), [`create_artifact`] reuses
    /// this id + its existing directory instead of minting a fresh
    /// UUID — so a Retry swaps the failed card in place rather than
    /// appending a second card (#3162). Unset for all normal
    /// generation paths, in which case a fresh UUID is minted.
    pub static REGENERATE_TARGET_ID: String;
}

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
/// When `thread_id` is `Some(_)` the listing is filtered to entries whose
/// persisted `meta.thread_id` matches verbatim (#3226); legacy meta.json
/// files without a `thread_id` field are excluded from the filtered set
/// because they have no addressable owning thread. The returned `total`
/// reflects the filtered count so the caller's pagination is per-thread,
/// not workspace-global.
///
/// Returns `(page, total)` where `page` is the requested slice and `total` is
/// the count before pagination (but after filtering).
pub(crate) async fn list_artifacts(
    workspace_dir: &Path,
    offset: usize,
    limit: usize,
    thread_id: Option<&str>,
) -> Result<(Vec<ArtifactMeta>, usize), String> {
    log::debug!(
        "[artifacts] list_artifacts: offset={offset} limit={limit} thread_id={:?} workspace={:?}",
        thread_id,
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

    // Apply thread filter BEFORE pagination so `total` reflects the
    // per-thread count the UI surfaces, and so a small page doesn't get
    // silently emptied by filtering after the slice (#3226).
    if let Some(tid) = thread_id {
        all.retain(|m| m.thread_id.as_deref() == Some(tid));
    }

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

/// Persist the verbatim producer-tool arguments alongside an artifact's
/// `meta.json` as `<workspace>/artifacts/<id>/args.json` (#3162).
///
/// Stored so a later [`ops::ai_regenerate`](super::ops::ai_regenerate)
/// can reload the exact spec and re-run generation deterministically —
/// the Retry affordance on a failed card re-dispatches the *same*
/// request rather than asking the LLM to reconstruct it. Best-effort
/// from the producer's perspective: a write failure here does not fail
/// the generation, it only forfeits the ability to regenerate that
/// artifact.
pub(crate) async fn save_artifact_args(
    workspace_dir: &Path,
    artifact_id: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    log::debug!("[artifacts] save_artifact_args: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(artifact_id);
    assert_within_root(&root, &artifact_dir)?;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(|e| {
            format!(
                "[artifacts] failed to create artifact dir {:?}: {e}",
                artifact_dir
            )
        })?;
    let args_path = artifact_dir.join(ARGS_FILENAME);
    let json = serde_json::to_string_pretty(args)
        .map_err(|e| format!("[artifacts] failed to serialize args for id={artifact_id}: {e}"))?;
    tokio::fs::write(&args_path, json)
        .await
        .map_err(|e| format!("[artifacts] failed to write args.json for id={artifact_id}: {e}"))?;
    log::debug!("[artifacts] saved args.json for id={artifact_id}");
    Ok(())
}

/// Load the persisted producer-tool arguments for an artifact (#3162).
///
/// Returns an `Err` when no `args.json` exists — the common case for
/// artifacts created before this sidecar was introduced, or by a
/// producer that never persisted args — so the caller can surface a
/// "cannot regenerate" message instead of silently doing nothing.
pub(crate) async fn read_artifact_args(
    workspace_dir: &Path,
    artifact_id: &str,
) -> Result<serde_json::Value, String> {
    log::debug!("[artifacts] read_artifact_args: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(artifact_id);
    assert_within_root(&root, &artifact_dir)?;
    let args_path = artifact_dir.join(ARGS_FILENAME);
    let contents = tokio::fs::read_to_string(&args_path).await.map_err(|e| {
        format!("[artifacts] no persisted args for id={artifact_id} (not regenerable): {e}")
    })?;
    serde_json::from_str(&contents)
        .map_err(|e| format!("[artifacts] corrupt args.json for id={artifact_id}: {e}"))
}

/// Read the raw bytes of a finalized artifact's output file.
///
/// Single source of truth for resolving an artifact id → on-disk bytes:
/// callers (e.g. the presentation image pipeline) must not reconstruct
/// the `<root>/<id>/<filename>` path scheme themselves. Validates the id,
/// confirms the resolved path stays under the artifacts root, and refuses
/// artifacts that are not yet [`ArtifactStatus::Ready`] (a `Pending` /
/// `Failed` record may have no bytes — or partial bytes — on disk).
pub async fn read_artifact_bytes(
    workspace_dir: &Path,
    artifact_id: &str,
) -> Result<Vec<u8>, String> {
    log::debug!("[artifacts] read_artifact_bytes: id={artifact_id}");
    let meta = get_artifact(workspace_dir, artifact_id).await?;
    if !matches!(meta.status, ArtifactStatus::Ready) {
        return Err(format!(
            "[artifacts] artifact id={artifact_id} is not ready (status={:?})",
            meta.status
        ));
    }
    let root = artifacts_root(workspace_dir).await?;
    // `meta.path` is the store-internal `<id>/<filename>` relative path.
    let file_path = root.join(&meta.path);
    assert_within_root(&root, &file_path)?;
    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("[artifacts] failed to read artifact bytes id={artifact_id}: {e}"))?;
    log::debug!(
        "[artifacts] read_artifact_bytes: id={artifact_id} read {} bytes",
        bytes.len()
    );
    Ok(bytes)
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
///
/// Publishes [`DomainEvent::ArtifactPending`] on the global bus the
/// moment the row is reserved so the chat surface can render an
/// in-progress / "Generating…" card immediately (#3162). When the
/// matching [`finalize_artifact`] / [`fail_artifact`] later fires it
/// reuses the same `artifact_id`, so the card swaps in place without
/// flicker. Same chat-context routing rules as the Ready/Failed pair —
/// `thread_id` / `client_id` come from the
/// [`crate::openhuman::approval::ApprovalChatContext`] task-local and
/// are `None` for CLI / cron / sub-agent paths, in which case the web
/// bridge silently drops the event for lack of a routing target.
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

    // Normal path mints a fresh UUID. A regenerate (#3162) runs inside
    // `REGENERATE_TARGET_ID.scope(...)` and reuses the original id so the
    // Pending/Ready/Failed events that follow carry the same artifact_id
    // and the card swaps in place. The reused dir already exists; the
    // `create_dir_all` below is idempotent and the meta/file are
    // overwritten with the fresh generation.
    let reused_id = REGENERATE_TARGET_ID
        .try_with(|target| target.clone())
        .ok()
        .filter(|target| !target.trim().is_empty());
    let is_regenerate = reused_id.is_some();
    let id = reused_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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

    // Capture the originating chat thread (if any) at create-time so the
    // panel can repopulate from disk after a redux-persist purge — see
    // #3226. `finalize_artifact` / `fail_artifact` already read the same
    // task-local for event publication; persisting it here means the
    // routing target survives a process restart.
    let (thread_id, _) = current_chat_context();

    // On a regenerate the id is reused in place, so preserve the original
    // `created_at` — bumping it to now would reorder the artifact to the
    // top of the `created_at`-sorted list/panel even though it is the same
    // logical artifact (#3162, CodeRabbit). New artifacts always stamp now.
    let created_at = if is_regenerate {
        match get_artifact(workspace_dir, &id).await {
            Ok(prev) => prev.created_at,
            Err(_) => chrono::Utc::now(),
        }
    } else {
        chrono::Utc::now()
    };

    let meta = ArtifactMeta {
        id: id.clone(),
        kind,
        title: trimmed_title.to_string(),
        path: relative_path,
        size_bytes: 0,
        status: ArtifactStatus::Pending,
        created_at,
        error: None,
        thread_id,
    };
    save_artifact_meta(workspace_dir, &meta).await?;

    log::debug!(
        "[artifacts] create_artifact: id={id} kind={} path={:?} thread_id={:?}",
        meta.kind.as_str(),
        absolute_path,
        meta.thread_id,
    );

    // Surface the "Generating…" card the moment the row is reserved so
    // the user doesn't stare at an empty composer until the tool finishes
    // (#3162). When `finalize_artifact` / `fail_artifact` later fires the
    // matching Ready/Failed event with the same `artifact_id`, the
    // frontend can swap the card in place.
    let (thread_id, client_id) = current_chat_context();
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::ArtifactPending {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        path: meta.path.clone(),
        thread_id,
        client_id,
    });

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
