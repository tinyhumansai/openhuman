use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

pub(crate) struct ResolvedNativeAttachment {
    pub(crate) path: String,
    pub(crate) mime: String,
    pub(crate) bytes: Vec<u8>,
}

pub(in crate::threads::attachments) const STAGED_DOMAIN: &str = "threads::attachments";
pub(in crate::threads::attachments) const STAGED_COLLECTION: &str =
    crate::threads::store::STAGED_ATTACHMENTS;
pub(super) const STAGED_TTL_SECS: i64 = 7 * 24 * 60 * 60;

const NATIVE_STAGED_ACTIVE: u8 = 0;
const NATIVE_STAGED_COMMITTED: u8 = 1;
const NATIVE_STAGED_DISCARDED: u8 = 2;
const NATIVE_STAGED_CLEANING: u8 = 3;

/// Owns staged refs from one native request until the durable transcript claim
/// succeeds or every queued request clone is dropped.
#[derive(Clone)]
pub(crate) struct NativeStagedLease(std::sync::Arc<NativeStagedLeaseState>);

struct NativeStagedLeaseState {
    dispatch: crate::core::runtime::DispatchCtx,
    thread_id: String,
    config: crate::config::Config,
    parts: std::sync::Mutex<Vec<ContentPart>>,
    state: AtomicU8,
}

impl NativeStagedLease {
    pub(crate) fn new(
        dispatch: crate::core::runtime::DispatchCtx,
        thread_id: String,
        config: crate::config::Config,
        parts: Vec<ContentPart>,
    ) -> Self {
        Self(std::sync::Arc::new(NativeStagedLeaseState {
            dispatch,
            thread_id,
            config,
            parts: std::sync::Mutex::new(parts),
            state: AtomicU8::new(NATIVE_STAGED_ACTIVE),
        }))
    }

    pub(crate) fn track(&self, part: ContentPart) -> Result<(), String> {
        let mut parts = self
            .0
            .parts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.0.state.load(Ordering::Acquire) != NATIVE_STAGED_ACTIVE {
            return Err("native attachment lease is no longer active".into());
        }
        parts.push(part);
        Ok(())
    }

    pub(crate) fn mark_committed(&self) {
        let _ = self.0.state.compare_exchange(
            NATIVE_STAGED_ACTIVE,
            NATIVE_STAGED_COMMITTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) async fn discard(&self) -> Result<(), String> {
        if self
            .0
            .state
            .compare_exchange(
                NATIVE_STAGED_ACTIVE,
                NATIVE_STAGED_CLEANING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Ok(());
        }
        let mut attempt = NativeStagedCleanupAttempt {
            state: self.0.clone(),
            completed: false,
        };
        let parts = self
            .0
            .parts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        discard_native_channel_parts(&self.0.dispatch, &self.0.thread_id, &self.0.config, &parts)
            .await?;
        self.0
            .state
            .store(NATIVE_STAGED_DISCARDED, Ordering::Release);
        attempt.completed = true;
        Ok(())
    }
}

struct NativeStagedCleanupAttempt {
    state: std::sync::Arc<NativeStagedLeaseState>,
    completed: bool,
}

impl Drop for NativeStagedCleanupAttempt {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.state.state.compare_exchange(
                NATIVE_STAGED_CLEANING,
                NATIVE_STAGED_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

impl Drop for NativeStagedLeaseState {
    fn drop(&mut self) {
        if self
            .state
            .compare_exchange(
                NATIVE_STAGED_ACTIVE,
                NATIVE_STAGED_DISCARDED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        let dispatch = self.dispatch.clone();
        let thread_id = self.thread_id.clone();
        let config = self.config.clone();
        let parts = self
            .parts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        dispatch.clone().spawn(async move {
            if discard_native_channel_parts(&dispatch, &thread_id, &config, &parts)
                .await
                .is_err()
            {
                tracing::warn!("[threads] queued native attachment cleanup failed");
            }
        });
    }
}

pub(crate) async fn stage_native_channel_part(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    input: &ContentPart,
    name: &str,
    mime: &str,
    bytes: &[u8],
    lease: &NativeStagedLease,
) -> Result<ContentPart, String> {
    if thread_id.trim().is_empty() || bytes.is_empty() {
        return Err("native attachment scope and bytes must be non-empty".into());
    }
    let mut staged_config = config.clone();
    staged_config.action_dir =
        crate::attachments::thread_action_dir(ctx, config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|error| format!("native attachment workspace unavailable: {error}"))?;
    let file = crate::attachments::store_scoped_channel_media(
        &staged_config,
        thread_id,
        name,
        mime,
        bytes,
    )
    .await
    .map_err(|_| "native attachment could not be staged safely".to_owned())?;
    let file_path = file.path.clone();
    let result = async {
        let mut part = input.clone();
        let media = opaque_media_mut(&mut part)
            .ok_or_else(|| "native attachment kind cannot be staged".to_owned())?;
        let opaque_id = uuid::Uuid::new_v4().simple().to_string();
        media.opaque_id = Some(opaque_id.clone());
        media.path = None;
        media.url = None;
        media.local_cache_path = None;
        media.content_type = Some(file.mime.clone());
        media.size_bytes = Some(file.size_bytes as u64);

        let tenant = tenant_for(ctx)?;
        let repo = staged_repo(ctx, &config.workspace_dir)?;
        let id = staged_doc_id(&opaque_id)?;
        let manifest = StagedAttachment {
            tenant,
            thread_id: thread_id.to_owned(),
            attachment: StoredAttachment {
                part: part.clone(),
                path: file.path,
            },
            expires_at: chrono::Utc::now().timestamp() + STAGED_TTL_SECS,
            claim: None,
        };
        let body = serde_json::to_value(&manifest)
            .map_err(|error| format!("serialize native attachment record: {error}"))?;
        repo.run(|docs| async move {
            docs.put(STAGED_COLLECTION, &id, body, Precondition::Absent)
                .await?;
            Ok(())
        })
        .map_err(|error| format!("persist native attachment record: {error}"))?;
        Ok(part)
    }
    .await;
    match result {
        Ok(part) => {
            lease.track(part.clone())?;
            Ok(part)
        }
        Err(error) => {
            let path = staged_config.action_dir.join(file_path);
            let _ = tokio::fs::remove_file(path).await;
            Err(error)
        }
    }
}

pub(crate) async fn commit_native_channel_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    parts: &[ContentPart],
    committed_message_id: &str,
) -> Result<(), String> {
    if parts.is_empty() {
        return Ok(());
    }
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("native attachment configuration unavailable: {error}"))?;
    let entries = read_staged_entries(ctx, thread_id, parts, &config).await?;
    validate_staged_files(ctx, thread_id, &config, &entries).await?;
    claim_staged_refs(ctx, thread_id, &config, committed_message_id, &entries).await
}

pub(crate) async fn discard_native_channel_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    parts: &[ContentPart],
) -> Result<(), String> {
    if parts.is_empty() {
        return Ok(());
    }
    let entries = read_staged_entries_for_cleanup(ctx, thread_id, parts, config).await?;
    discard_staged_entries(ctx, thread_id, config, entries).await
}

pub(crate) async fn read_authorized_native_attachment(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    thread_id: &str,
    opaque_id: &str,
    current_parts: &[ContentPart],
    allow_committed_history: bool,
) -> Result<ResolvedNativeAttachment, String> {
    let id = staged_doc_id(opaque_id)?;
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    let record = get_staged_record(&repo, &id)?
        .ok_or_else(|| "native attachment is not registered".to_owned())?;
    if record.tenant != tenant || record.thread_id != thread_id {
        return Err("native attachment is outside the active session scope".into());
    }
    if record.claim.as_ref().is_some_and(|claim| claim.canceled) {
        return Err("native attachment was canceled".into());
    }
    let current_authority = current_parts.iter().any(|part| {
        opaque_media(part)
            .and_then(|media| media.opaque_id.as_deref())
            .is_some_and(|candidate| candidate == opaque_id)
            && part == &record.attachment.part
    });
    let committed_authority = record.claim.as_ref().is_some_and(|claim| {
        claim.tenant == tenant
            && claim.thread_id == thread_id
            && claim.part == record.attachment.part
    });
    if !(current_authority || allow_committed_history && committed_authority) {
        return Err("native attachment lacks a current or committed host grant".into());
    }
    validate_native_attachment_policy(&record.attachment, config)?;
    let entries = vec![record.attachment.clone()];
    validate_staged_files(ctx, thread_id, config, &entries).await?;
    let entry = &record.attachment;
    let workspace =
        crate::attachments::thread_action_dir(ctx, config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|_| "native attachment workspace is unavailable".to_owned())?;
    let scope = crate::attachments::AttachmentAccessScope {
        external_channel: false,
        workspace: Some(workspace.clone()),
        thread_id: Some(thread_id.to_owned()),
        ..Default::default()
    };
    let path = crate::attachments::resolve_path(config, &entry.path, &scope)
        .await
        .map_err(|_| "native attachment workspace path is invalid".to_owned())?;
    let media =
        opaque_media(&entry.part).ok_or_else(|| "native attachment kind is invalid".to_owned())?;
    let mime = media
        .content_type
        .clone()
        .ok_or_else(|| "native attachment MIME is unavailable".to_owned())?;
    let root = tokio::fs::canonicalize(&workspace)
        .await
        .map_err(|_| "native attachment workspace is unavailable".to_owned())?;
    let limit = native_attachment_byte_limit(&record.attachment.part, config);
    let bytes = read_staged_file_bounded(&path, &root, limit).await?;
    validate_native_attachment_size(&record.attachment.part, bytes.len(), config)?;
    Ok(ResolvedNativeAttachment {
        path: entry.path.clone(),
        mime,
        bytes,
    })
}

fn native_attachment_byte_limit(part: &ContentPart, config: &crate::config::Config) -> usize {
    let is_image = matches!(part, ContentPart::Image { .. });
    let mb = if is_image {
        config.multimodal.effective_limits().1
    } else {
        config.multimodal_files.effective_limits().1
    };
    mb.saturating_mul(1024 * 1024)
        .min(tinychannels_bus::module_media::MEDIA_UPLOAD_MAX_BYTES)
}

pub(in crate::threads::attachments) async fn read_staged_file_bounded(
    path: &std::path::Path,
    root: &std::path::Path,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let file = crate::security::secure_open::secure_open(path, root)
        .map_err(|_| "native attachment bytes are unavailable".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "native attachment bytes are unavailable".to_owned())?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("native attachment exceeds its configured byte limit".into());
    }
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::from_std(file);
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "native attachment bytes are unavailable".to_owned())?;
    if bytes.len() > limit {
        return Err("native attachment exceeds its configured byte limit".into());
    }
    Ok(bytes)
}

fn validate_native_attachment_policy(
    entry: &StoredAttachment,
    config: &crate::config::Config,
) -> Result<(), String> {
    let media =
        opaque_media(&entry.part).ok_or_else(|| "native attachment kind is invalid".to_owned())?;
    let mime = media
        .content_type
        .as_deref()
        .ok_or_else(|| "native attachment MIME is unavailable".to_owned())?;
    let is_image = matches!(entry.part, ContentPart::Image { .. });
    let allowed = if is_image {
        matches!(
            mime,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        )
    } else {
        config
            .multimodal_files
            .allowed_mime_types
            .iter()
            .any(|value| value == mime)
    };
    if !allowed {
        return Err("native attachment MIME is no longer allowed".into());
    }
    Ok(())
}

fn validate_native_attachment_size(
    part: &ContentPart,
    actual_bytes: usize,
    config: &crate::config::Config,
) -> Result<(), String> {
    let media = opaque_media(part).ok_or_else(|| "native attachment kind is invalid".to_owned())?;
    let is_image = matches!(part, ContentPart::Image { .. });
    let declared = media
        .size_bytes
        .and_then(|size| usize::try_from(size).ok())
        .ok_or_else(|| "native attachment size is unavailable".to_owned())?;
    let (image_mb, file_mb) = (
        config.multimodal.effective_limits().1,
        config.multimodal_files.effective_limits().1,
    );
    let limit = (if is_image { image_mb } else { file_mb })
        .saturating_mul(1024 * 1024)
        .min(tinychannels_bus::module_media::MEDIA_UPLOAD_MAX_BYTES);
    if actual_bytes == 0 || actual_bytes != declared || actual_bytes > limit {
        return Err("native attachment exceeds its configured byte limit".into());
    }
    Ok(())
}

pub(crate) fn native_parts_in_committed_message(
    staged: &[ContentPart],
    message: &tinyinference_llm::message::Message,
) -> Vec<ContentPart> {
    let tinyinference_llm::message::Message::User(user) = message else {
        return Vec::new();
    };
    let committed = user
        .content
        .iter()
        .filter_map(|block| match block {
            tinyinference_llm::message::ContentBlock::Image(image) => {
                image.url.strip_prefix("openhuman-staged://")
            }
            tinyinference_llm::message::ContentBlock::Document(
                tinyinference_llm::message::MediaRef::Path { path, .. },
            ) => path.strip_prefix("openhuman-staged://"),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    staged
        .iter()
        .filter(|part| {
            opaque_media(part)
                .and_then(|media| media.opaque_id.as_deref())
                .is_some_and(|id| committed.contains(id))
        })
        .cloned()
        .collect()
}
