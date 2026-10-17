use std::path::{Path, PathBuf};

use super::files::{self, FileRoots};
use super::store_documents::{self, documents, on_docs, page_of};
use super::types::{ArtifactMeta, ArtifactStatus};
use crate::security::approval::ApprovalChatContext;

const ARTIFACTS_SUBDIR: &str = "artifacts";
const META_FILENAME: &str = "meta.json";
/// Sidecar file holding the verbatim producer-tool arguments that
/// generated the artifact, persisted next to `meta.json` so a failed
/// card's Retry button can re-dispatch the exact same generation
/// deterministically without round-tripping the args back through the
/// LLM (#3162). Written by the producing tool right after
/// [`create_artifact`]; read by `ops::ai_regenerate`.
const ARGS_FILENAME: &str = "args.json";

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
    let docs = documents(workspace_dir).await?;
    let owned = meta.clone();
    on_docs(docs, move |docs| docs.put_meta(&owned)).await
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
    let docs = documents(workspace_dir).await?;
    let owned = thread_id.map(str::to_string);
    let all = on_docs(docs, move |docs| match owned {
        Some(thread_id) => docs.list_meta_for_thread(&thread_id),
        None => docs.list_meta(),
    })
    .await?;
    Ok(page_of(all, offset, limit, thread_id))
}

/// Retrieve a single artifact by ID.
pub(crate) async fn get_artifact(
    workspace_dir: &Path,
    artifact_id: &str,
) -> Result<ArtifactMeta, String> {
    log::debug!("[artifacts] get_artifact: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let docs = documents(workspace_dir).await?;
    let id = artifact_id.to_string();
    on_docs(docs, move |docs| docs.get_meta(&id))
        .await?
        .ok_or_else(|| format!("[artifacts] artifact not found id={artifact_id}"))
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
    let docs = documents(workspace_dir).await?;
    let (id, owned) = (artifact_id.to_string(), args.clone());
    on_docs(docs, move |docs| docs.put_args(&id, &owned)).await
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
    let docs = documents(workspace_dir).await?;
    let id = artifact_id.to_string();
    on_docs(docs, move |docs| docs.get_args(&id))
        .await?
        .ok_or_else(|| {
            format!("[artifacts] no persisted args for id={artifact_id} (not regenerable)")
        })
}

/// Read the raw bytes of a finalized artifact's output file.
///
/// Single source of truth for resolving an artifact id → on-disk bytes:
/// callers (e.g. the presentation image pipeline) must not reconstruct the
/// path themselves. Goes through [`files::resolve_ready_file`], which refuses
/// records that are not yet [`ArtifactStatus::Ready`] (a `Pending` / `Failed`
/// record may have no bytes — or partial bytes — on disk), applies the escape
/// guard, and reports a file removed outside OpenHuman as missing.
pub async fn read_artifact_bytes(
    workspace_dir: &Path,
    roots: impl Into<FileRoots>,
    artifact_id: &str,
) -> Result<Vec<u8>, String> {
    log::debug!("[artifacts] read_artifact_bytes: id={artifact_id}");
    let file_path = files::resolve_ready_file(workspace_dir, &roots.into(), artifact_id).await?;
    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("[artifacts] failed to read artifact bytes id={artifact_id}: {e}"))?;
    log::debug!(
        "[artifacts] read_artifact_bytes: id={artifact_id} read {} bytes",
        bytes.len()
    );
    Ok(bytes)
}

/// Delete an artifact: its file in the files folder, then its metadata
/// directory. A file already gone, or one the escape guard rejects, is left
/// alone (logged) and the record is still removed.
pub(crate) async fn delete_artifact(
    workspace_dir: &Path,
    roots: impl Into<FileRoots>,
    artifact_id: &str,
) -> Result<(), String> {
    let roots = roots.into();
    log::debug!("[artifacts] delete_artifact: id={artifact_id}");
    validate_artifact_id(artifact_id)?;
    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(artifact_id);
    assert_within_root(&root, &artifact_dir)?;
    if let Ok(meta) = get_artifact(workspace_dir, artifact_id).await {
        if meta.file.is_some() {
            match files::resolve_file(workspace_dir, &meta, &roots).await {
                Ok(file) => match tokio::fs::remove_file(&file).await {
                    Ok(()) => log::debug!(
                        "[artifacts] delete_artifact: removed file for id={artifact_id}"
                    ),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(format!(
                            "[artifacts] failed to delete file for id={artifact_id}: {e}"
                        ))
                    }
                },
                Err(e) => log::warn!("[artifacts] delete_artifact: leaving file in place: {e}"),
            }
        }
    }
    let docs = documents(workspace_dir).await?;
    store_documents::delete_record(docs, &artifact_dir, artifact_id).await
}

// Mark a status as unused — referenced only in tests via the store

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
/// `thread_id` / `client_id` come from the `chat` argument (the producing
/// turn's [`crate::security::approval::ApprovalChatContext`]) and
/// are `None` for CLI / cron / sub-agent paths, in which case the web
/// bridge silently drops the event for lack of a routing target.
pub async fn create_artifact(
    workspace_dir: &Path,
    files: impl Into<FileRoots>,
    kind: super::types::ArtifactKind,
    title: &str,
    extension: &str,
    chat: Option<&ApprovalChatContext>,
) -> Result<(ArtifactMeta, PathBuf), String> {
    create_artifact_for_call(workspace_dir, files, kind, title, extension, None, chat).await
}

/// As [`create_artifact`], but also records the provider-assigned
/// `tool_call_id` of the producing invocation (typically
/// `crate::tools::host_extensions::tool_call_id(ctx)`) on the artifact's
/// metadata and on the `ArtifactPending` event this publishes, so the UI
/// can correlate the card with the tool-call bubble. `None` behaves
/// exactly like [`create_artifact`].
pub async fn create_artifact_for_call(
    workspace_dir: &Path,
    files: impl Into<FileRoots>,
    kind: super::types::ArtifactKind,
    title: &str,
    extension: &str,
    tool_call_id: Option<&str>,
    chat: Option<&ApprovalChatContext>,
) -> Result<(ArtifactMeta, PathBuf), String> {
    create_artifact_regenerating(
        workspace_dir,
        files,
        kind,
        title,
        extension,
        tool_call_id,
        None,
        chat,
    )
    .await
}

/// As [`create_artifact_for_call`], but a `regenerate_target` (the id
/// `ops::ai_regenerate` re-dispatches a failed card under) makes the call
/// reuse that id and its existing record instead of minting a fresh UUID, so a
/// Retry swaps the failed card in place rather than appending a second one
/// (#3162). The target travels as an explicit argument from the regenerating
/// caller to the tool that produces the artifact. A blank target mints a fresh
/// id like `None`.
pub async fn create_artifact_regenerating(
    workspace_dir: &Path,
    files: impl Into<FileRoots>,
    kind: super::types::ArtifactKind,
    title: &str,
    extension: &str,
    tool_call_id: Option<&str>,
    regenerate_target: Option<&str>,
    chat: Option<&ApprovalChatContext>,
) -> Result<(ArtifactMeta, PathBuf), String> {
    let roots = files.into();
    let files_dir = roots.current();
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
    // an explicit `regenerate_target` and reuses the original id so the
    // Pending/Ready/Failed events that follow carry the same artifact_id
    // and the card swaps in place. The reused dir already exists; the
    // `create_dir_all` below is idempotent and the meta/file are
    // overwritten with the fresh generation.
    let reused_id = regenerate_target
        .map(str::to_owned)
        .filter(|target| !target.trim().is_empty());
    let is_regenerate = reused_id.is_some();
    let id = reused_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let stem = sanitize_filename_stem(trimmed_title);

    let root = artifacts_root(workspace_dir).await?;
    let artifact_dir = root.join(&id);
    assert_within_root(&root, &artifact_dir)?;
    // The record is a document, so there is no per-artifact directory to make.

    // Capture the originating chat thread (if any) at create-time so the
    // panel can repopulate from disk after a redux-persist purge — see
    // #3226. `finalize_artifact` / `fail_artifact` already read the same
    // chat context for event publication; persisting it here means the
    // routing target survives a process restart.
    let (thread_id, _, _) = chat_route(chat);

    // On a regenerate the id is reused in place, so preserve the original
    // `created_at` — bumping it to now would reorder the artifact to the
    // top of the `created_at`-sorted list/panel even though it is the same
    // logical artifact (#3162, CodeRabbit). New artifacts always stamp now.
    // A regenerate also overwrites the file it already owns (#5505) rather
    // than claiming a second name in the files folder.
    let previous = if is_regenerate {
        get_artifact(workspace_dir, &id).await.ok()
    } else {
        None
    };
    let created_at = previous
        .as_ref()
        .map(|prev| prev.created_at)
        .unwrap_or_else(chrono::Utc::now);
    let owned_file = match previous.as_ref().filter(|prev| prev.file.is_some()) {
        Some(prev) => match files::resolve_file(workspace_dir, prev, &roots).await {
            Ok(path) if tokio::fs::try_exists(&path).await.unwrap_or(false) => prev
                .file_root
                .clone()
                .map(|root| (path, PathBuf::from(root))),
            _ => None,
        },
        None => None,
    };
    let (absolute_path, file_root) = match owned_file {
        Some(pair) => pair,
        None => (
            files::reserve_file(files_dir, &stem, trimmed_ext, &id).await?,
            files_dir.to_path_buf(),
        ),
    };
    let filename = absolute_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{stem}.{trimmed_ext}"));

    let meta = ArtifactMeta {
        id: id.clone(),
        kind,
        title: trimmed_title.to_string(),
        path: filename,
        file: Some(absolute_path.to_string_lossy().into_owned()),
        file_root: Some(file_root.to_string_lossy().into_owned()),
        size_bytes: 0,
        status: ArtifactStatus::Pending,
        created_at,
        error: None,
        thread_id,
        // A regenerate keeps the tool call that made the artifact: dropping it
        // would detach the card from that call's bubble.
        tool_call_id: tool_call_id
            .map(str::to_string)
            .or_else(|| previous.as_ref().and_then(|prev| prev.tool_call_id.clone())),
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
    let (thread_id, client_id, request_id) = chat_route(chat);
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ArtifactPending {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        path: meta.path.clone(),
        thread_id,
        client_id,
        tool_call_id: meta.tool_call_id.clone(),
        request_id,
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
/// When the caller passes no
/// [`ApprovalChatContext`](crate::security::approval::ApprovalChatContext)
/// (CLI / cron / sub-agent paths), the event is still published but
/// `thread_id` / `client_id` are `None` so the socket bridge silently
/// drops it. Idempotent calls (already-Ready) skip the publish so we
/// don't flap the UI.
pub async fn finalize_artifact(
    workspace_dir: &Path,
    artifact_id: &str,
    size_bytes: u64,
    chat: Option<&ApprovalChatContext>,
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

    let (thread_id, client_id, request_id) = chat_route(chat);
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ArtifactReady {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        path: meta.path.clone(),
        size_bytes: meta.size_bytes,
        thread_id,
        client_id,
        tool_call_id: meta.tool_call_id.clone(),
        request_id,
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
    roots: impl Into<FileRoots>,
    artifact_id: &str,
    reason: &str,
    chat: Option<&ApprovalChatContext>,
) -> Result<ArtifactMeta, String> {
    let mut meta = get_artifact(workspace_dir, artifact_id).await?;
    meta.status = ArtifactStatus::Failed;
    meta.error = Some(reason.to_string());
    save_artifact_meta(workspace_dir, &meta).await?;
    files::remove_empty_placeholder(workspace_dir, &meta, &roots.into()).await;
    // Log only the size of the reason — it can carry provider stderr
    // / user-derived content, which we don't want flushed verbatim
    // into structured logs. The full payload is still persisted on
    // `meta.error` for the UI surface and the chat event below.
    log::warn!(
        "[artifacts] fail_artifact: id={artifact_id} -> Failed reason_len={}",
        reason.len()
    );

    let (thread_id, client_id, request_id) = chat_route(chat);
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ArtifactFailed {
        artifact_id: meta.id.clone(),
        kind: meta.kind.as_str().to_string(),
        title: meta.title.clone(),
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        error: reason.to_string(),
        thread_id,
        client_id,
        tool_call_id: meta.tool_call_id.clone(),
        request_id,
    });
    Ok(meta)
}

/// The routing triple of the producing turn's [`ApprovalChatContext`]: its
/// thread id, client id and originating `request_id`. `(None, None, None)` for
/// non-chat callers (CLI, cron, sub-agent runners) so artifact emit hooks
/// degrade gracefully: the event is still published but the web subscriber
/// drops it for lack of a routing target.
fn chat_route(
    chat: Option<&ApprovalChatContext>,
) -> (Option<String>, Option<String>, Option<String>) {
    match chat {
        Some(ctx) => (
            Some(ctx.thread_id.clone()),
            Some(ctx.client_id.clone()),
            ctx.request_id.clone(),
        ),
        None => (None, None, None),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
