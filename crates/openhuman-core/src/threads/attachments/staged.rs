use super::*;
use crate::storage::documents::compare_and_swap;
use tinystoragedrivers::CollectionSpec;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StagedAttachment {
    pub(super) tenant: crate::core::runtime::Tenant,
    pub(super) thread_id: String,
    pub(super) attachment: StoredAttachment,
    pub(super) expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) claim: Option<StagedAttachmentClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct StagedAttachmentClaim {
    pub(super) tenant: crate::core::runtime::Tenant,
    pub(super) thread_id: String,
    pub(super) message_id: String,
    pub(super) part: ContentPart,
    #[serde(default)]
    pub(super) canceled: bool,
}

pub(crate) use native::{
    commit_native_channel_parts, native_parts_in_committed_message,
    read_authorized_native_attachment, stage_native_channel_part, NativeStagedLease,
};
#[path = "staged/native.rs"]
mod native;
#[cfg(test)]
pub(super) use native::read_staged_file_bounded;
use native::STAGED_TTL_SECS;
pub(super) use native::{STAGED_COLLECTION, STAGED_DOMAIN};

pub(super) fn staged_doc_id(opaque_id: &str) -> Result<String, String> {
    let uuid = uuid::Uuid::parse_str(opaque_id)
        .map_err(|_| "staged attachment reference is invalid".to_string())?;
    Ok(uuid.simple().to_string())
}

pub(super) fn staged_collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(STAGED_COLLECTION)]
}

pub(super) fn staged_repo(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &std::path::Path,
) -> Result<Repo, String> {
    let tenant = tenant_for(ctx)?;
    let backend = storage::local::backend_for(workspace)
        .map_err(|error| format!("open staged attachment storage: {error}"))?;
    let scope = storage::scope_from(
        tenant.profile.as_deref(),
        tenant.agent.as_deref(),
        crate::core::runtime::mode::is_saas(),
    )
    .map_err(|error| format!("scope staged attachment storage: {error}"))?;
    Repo::on(&backend, &scope, STAGED_DOMAIN, staged_collections)
        .map_err(|error| format!("open staged attachment collection: {error}"))
}

pub(super) fn get_staged_record(repo: &Repo, id: &str) -> Result<Option<StagedAttachment>, String> {
    let id = id.to_string();
    repo.run(|docs| async move {
        let Some(stored) = docs.get(STAGED_COLLECTION, &id).await? else {
            return Ok(None);
        };
        serde_json::from_value(stored.doc)
            .map(Some)
            .map_err(|error| tinystoragedrivers::StorageError::invalid_input(error.to_string()))
    })
    .map_err(|error| error.to_string())
}

pub(super) async fn read_staged_entries(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    requested: &[ContentPart],
    config: &crate::config::Config,
) -> Result<Vec<StoredAttachment>, String> {
    read_staged_entries_with_canceled_cleanup(ctx, thread_id, requested, config, false).await
}

pub(super) async fn read_staged_entries_for_cleanup(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    requested: &[ContentPart],
    config: &crate::config::Config,
) -> Result<Vec<StoredAttachment>, String> {
    read_staged_entries_with_canceled_cleanup(ctx, thread_id, requested, config, true).await
}

async fn read_staged_entries_with_canceled_cleanup(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    requested: &[ContentPart],
    config: &crate::config::Config,
    allow_canceled_cleanup: bool,
) -> Result<Vec<StoredAttachment>, String> {
    validate_opaque_parts(requested)?;
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    let mut entries = Vec::with_capacity(requested.len());
    for part in requested {
        let opaque_id = opaque_media(part)
            .and_then(|media| media.opaque_id.as_deref())
            .ok_or_else(|| "staged attachment reference is missing".to_string())?;
        let id = staged_doc_id(opaque_id)?;
        let manifest = get_staged_record(&repo, &id)?
            .ok_or_else(|| "staged attachment is missing or expired".to_string())?;
        if manifest.tenant != tenant || manifest.thread_id != thread_id {
            return Err("staged attachment belongs to a different scope".into());
        }
        if manifest.expires_at < chrono::Utc::now().timestamp()
            && manifest.claim.as_ref().is_none_or(|claim| claim.canceled)
            && !allow_canceled_cleanup
        {
            return Err("staged attachment is missing or expired".into());
        }
        if manifest.claim.as_ref().is_some_and(|claim| claim.canceled) && !allow_canceled_cleanup {
            return Err("staged attachment was canceled".into());
        }
        if &manifest.attachment.part != part {
            return Err("staged attachment reference does not match its stored record".into());
        }
        entries.push(manifest.attachment);
    }
    Ok(entries)
}

/// Bind staged refs to a new user row's private path map without exposing
/// paths in the returned row projection.
pub(crate) async fn bind_staged_refs(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    requested: &[ContentPart],
    message_id: &str,
    metadata: &mut Value,
) -> Result<Vec<ContentPart>, String> {
    let entries = read_staged_entries(ctx, thread_id, requested, config).await?;
    validate_staged_files(ctx, thread_id, config, &entries).await?;
    claim_staged_refs(ctx, thread_id, config, message_id, &entries).await?;
    Ok(save(
        metadata,
        entries
            .into_iter()
            .map(|entry| (entry.part, entry.path))
            .collect(),
    ))
}

pub(super) async fn claim_staged_refs(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    message_id: &str,
    entries: &[StoredAttachment],
) -> Result<(), String> {
    if message_id.is_empty() {
        return Err("staged attachment target message ID is empty".into());
    }
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    for entry in entries {
        let opaque_id = opaque_media(&entry.part)
            .and_then(|media| media.opaque_id.as_deref())
            .ok_or_else(|| "staged attachment reference is missing".to_string())?;
        let id = staged_doc_id(opaque_id)?;
        let claim = StagedAttachmentClaim {
            tenant: tenant.clone(),
            thread_id: thread_id.to_string(),
            message_id: message_id.to_string(),
            part: entry.part.clone(),
            canceled: false,
        };
        claim_staged_record(&repo, &tenant, thread_id, &id, &entry.part, &claim)?;
    }
    Ok(())
}

pub(super) fn claim_staged_record(
    repo: &Repo,
    tenant: &crate::core::runtime::Tenant,
    thread_id: &str,
    id: &str,
    part: &ContentPart,
    claim: &StagedAttachmentClaim,
) -> Result<bool, String> {
    let id = id.to_string();
    let tenant = tenant.clone();
    let thread_id = thread_id.to_string();
    let part = part.clone();
    let claim = claim.clone();
    repo.run(|docs| async move {
        let updated = compare_and_swap(&docs, STAGED_COLLECTION, &id, |doc| {
            let mut record: StagedAttachment = serde_json::from_value(doc.clone()).ok()?;
            if record.tenant != tenant
                || record.thread_id != thread_id
                || record.attachment.part != part
                || record.expires_at < chrono::Utc::now().timestamp()
            {
                return None;
            }
            if record.claim.is_some() {
                return None;
            }
            record.claim = Some(claim.clone());
            serde_json::to_value(record).ok()
        })
        .await?;
        if updated.is_some() {
            return Ok(true);
        }
        let Some(stored) = docs.get(STAGED_COLLECTION, &id).await? else {
            return Err(tinystoragedrivers::StorageError::not_found(
                "staged attachment is missing or expired",
            ));
        };
        let record: StagedAttachment = serde_json::from_value(stored.doc)
            .map_err(|error| tinystoragedrivers::StorageError::invalid_input(error.to_string()))?;
        if record.tenant != tenant
            || record.thread_id != thread_id
            || record.attachment.part != part
        {
            return Err(tinystoragedrivers::StorageError::conflict(
                "staged attachment belongs to a different scope",
            ));
        }
        if record.claim.as_ref() == Some(&claim) {
            return Ok(false);
        }
        if record
            .claim
            .as_ref()
            .is_some_and(|existing| existing.canceled)
        {
            return Err(tinystoragedrivers::StorageError::conflict(
                "staged attachment was canceled before it could be bound",
            ));
        }
        Err(tinystoragedrivers::StorageError::conflict(
            "staged attachment is already bound to another message",
        ))
    })
    .map_err(|error| error.to_string())
}

/// Stage media supplied with a queued follow-up, returning only opaque refs.
/// The manifest is bound to this dispatch tenant and thread, and remains
/// separate from visible message history until the runner admits the turn.
pub(crate) async fn stage_queued_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    input: &[ContentPart],
) -> Result<Vec<ContentPart>, String> {
    if thread_id.is_empty() || input.is_empty() {
        return Err("queued attachment scope and parts must be non-empty".into());
    }
    for part in input {
        if let Some(media) = opaque_media(part) {
            if media.opaque_id.is_some() {
                return Err("queued attachment input must not contain existing opaque refs".into());
            }
        }
    }
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("queued attachment configuration unavailable: {error}"))?;
    let workspace =
        crate::attachments::thread_action_dir(ctx, &config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|error| format!("queued attachment workspace unavailable: {error}"))?;
    let origin = ctx.turn_origin_or_scoped();
    let folded = crate::attachments::parts::fold_into_message("", input)
        .map_err(|error| error.to_string())?;
    let staged = crate::attachments::stage_turn(
        &folded,
        Some(&config),
        Some(&tinytools::WorkspaceDescriptor::new(workspace)),
        Some(thread_id),
        origin.as_ref(),
    )
    .await
    .map_err(|error| format!("stage queued attachments: {error}"))?;
    let (_, files) = crate::attachments::parse(&staged);
    let entries = from_staged_parts(input, &files)?;
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    let expires_at = chrono::Utc::now().timestamp() + STAGED_TTL_SECS;
    let mut public = Vec::with_capacity(entries.len());
    for attachment in entries {
        let opaque_id = opaque_media(&attachment.0)
            .and_then(|media| media.opaque_id.as_deref())
            .ok_or_else(|| "staged attachment has no opaque reference".to_string())?;
        let id = staged_doc_id(opaque_id)?;
        let manifest = StagedAttachment {
            tenant: tenant.clone(),
            thread_id: thread_id.to_string(),
            attachment: StoredAttachment {
                part: attachment.0.clone(),
                path: attachment.1,
            },
            expires_at,
            claim: None,
        };
        let body = serde_json::to_value(&manifest)
            .map_err(|error| format!("serialize queued attachment record: {error}"))?;
        repo.run(|docs| async move {
            docs.put(STAGED_COLLECTION, &id, body, Precondition::Absent)
                .await?;
            Ok(())
        })
        .map_err(|error| format!("persist queued attachment record: {error}"))?;
        public.push(manifest.attachment.part);
    }
    Ok(public)
}

/// Store bytes obtained from a trusted, scoped channel-module media read as a
/// staged attachment. The provider reference is replaced with a fresh opaque
/// host reference; the private workspace path remains only in this record.
pub(super) async fn validate_staged_files(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    entries: &[StoredAttachment],
) -> Result<(), String> {
    let workspace =
        crate::attachments::thread_action_dir(ctx, config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|error| format!("staged attachment workspace unavailable: {error}"))?;
    let image_count = entries
        .iter()
        .filter(|entry| matches!(entry.part, ContentPart::Image { .. }))
        .count();
    let file_count = entries.len() - image_count;
    let (max_images, image_mb) = config.multimodal.effective_limits();
    let (max_files, file_mb, _) = config.multimodal_files.effective_limits();
    if (image_count > 0 && (config.multimodal.max_images == 0 || image_count > max_images))
        || (!entries.is_empty() && config.multimodal_files.max_files == 0)
        || file_count > max_files
    {
        return Err("staged attachment count exceeds configured limit".into());
    }
    for entry in entries {
        let scope = crate::attachments::AttachmentAccessScope {
            external_channel: false,
            workspace: Some(workspace.clone()),
            thread_id: Some(thread_id.to_owned()),
            ..Default::default()
        };
        let path = crate::attachments::resolve_path(config, &entry.path, &scope)
            .await
            .map_err(|error| format!("staged attachment is unavailable: {error}"))?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| format!("staged attachment is unavailable: {error}"))?;
        let media = opaque_media(&entry.part)
            .ok_or_else(|| "staged attachment has an unsupported kind".to_string())?;
        let is_image = matches!(entry.part, ContentPart::Image { .. });
        let mime = media
            .content_type
            .as_deref()
            .filter(|mime| !mime.is_empty())
            .ok_or_else(|| "staged attachment MIME type is missing".to_string())?;
        if (is_image && !mime.starts_with("image/"))
            || !metadata.is_file()
            || metadata.len() > (if is_image { image_mb } else { file_mb } * 1024 * 1024) as u64
        {
            return Err("staged attachment exceeds configured type or size limit".into());
        }
    }
    Ok(())
}

/// Remove staged media for a queue entry that was canceled before execution.
pub(crate) async fn discard_staged_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    requested: &[ContentPart],
) -> Result<(), String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("staged attachment configuration unavailable: {error}"))?;
    let entries = read_staged_entries(ctx, thread_id, requested, &config).await?;
    discard_staged_entries(ctx, thread_id, &config, entries).await
}

pub(super) async fn discard_staged_entries(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    config: &crate::config::Config,
    entries: Vec<StoredAttachment>,
) -> Result<(), String> {
    let workspace =
        crate::attachments::thread_action_dir(ctx, config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|error| format!("staged attachment workspace unavailable: {error}"))?;
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    for entry in entries {
        let Some(id) = opaque_media(&entry.part).and_then(|media| media.opaque_id.as_deref())
        else {
            continue;
        };
        let id = staged_doc_id(id)?;
        let cancellation = StagedAttachmentClaim {
            tenant: tenant.clone(),
            thread_id: thread_id.to_string(),
            message_id: String::new(),
            part: entry.part.clone(),
            canceled: true,
        };
        if !cancel_staged_record(&repo, &tenant, thread_id, &id, &entry.part, &cancellation)? {
            continue;
        }
        let scope = crate::attachments::AttachmentAccessScope {
            external_channel: false,
            workspace: Some(workspace.clone()),
            thread_id: Some(thread_id.to_owned()),
            ..Default::default()
        };
        let path = crate::attachments::resolve_path(config, &entry.path, &scope)
            .await
            .map_err(|error| format!("staged attachment cleanup refused: {error}"))?;
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove staged attachment: {error}")),
        }
    }
    Ok(())
}

pub(super) fn cancel_staged_record(
    repo: &Repo,
    tenant: &crate::core::runtime::Tenant,
    thread_id: &str,
    id: &str,
    part: &ContentPart,
    cancellation: &StagedAttachmentClaim,
) -> Result<bool, String> {
    let id = id.to_string();
    let tenant = tenant.clone();
    let thread_id = thread_id.to_string();
    let part = part.clone();
    let cancellation = cancellation.clone();
    repo.run(|docs| async move {
        let updated = compare_and_swap(&docs, STAGED_COLLECTION, &id, |doc| {
            let mut record: StagedAttachment = serde_json::from_value(doc.clone()).ok()?;
            if record.tenant != tenant
                || record.thread_id != thread_id
                || record.attachment.part != part
            {
                return None;
            }
            if record.claim.is_some() {
                return None;
            }
            record.claim = Some(cancellation.clone());
            serde_json::to_value(record).ok()
        })
        .await?;
        if updated.is_some() {
            return Ok(true);
        }
        let Some(stored) = docs.get(STAGED_COLLECTION, &id).await? else {
            return Err(tinystoragedrivers::StorageError::not_found(
                "staged attachment is missing",
            ));
        };
        let record: StagedAttachment = serde_json::from_value(stored.doc)
            .map_err(|error| tinystoragedrivers::StorageError::invalid_input(error.to_string()))?;
        if record.tenant != tenant
            || record.thread_id != thread_id
            || record.attachment.part != part
        {
            return Err(tinystoragedrivers::StorageError::conflict(
                "staged attachment belongs to a different scope",
            ));
        }
        match record.claim {
            Some(existing) if existing == cancellation => Ok(true),
            Some(existing) if existing.canceled => Ok(false),
            Some(_) => Ok(false),
            None => Err(tinystoragedrivers::StorageError::conflict(
                "staged attachment cancellation raced with another change",
            )),
        }
    })
    .map_err(|error| error.to_string())
}

/// Retire manifests after the staged refs have been durably bound to a
/// message row. Files remain at their validated paths and are now owned by
/// that row's private attachment metadata.
pub(crate) async fn consume_staged_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    requested: &[ContentPart],
    message_id: &str,
) -> Result<(), String> {
    if requested.is_empty() {
        return Ok(());
    }
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("staged attachment configuration unavailable: {error}"))?;
    let tenant = tenant_for(ctx)?;
    let repo = staged_repo(ctx, &config.workspace_dir)?;
    for part in requested {
        let id = opaque_media(part)
            .and_then(|media| media.opaque_id.as_deref())
            .ok_or_else(|| "staged attachment reference is missing".to_string())?;
        let record = get_staged_record(&repo, &staged_doc_id(id)?)?
            .ok_or_else(|| "staged attachment is not bound to a persisted message".to_string())?;
        let Some(claim) = record.claim else {
            return Err("staged attachment is not bound to a persisted message".into());
        };
        if record.tenant != tenant
            || record.thread_id != thread_id
            || claim.tenant != tenant
            || claim.thread_id != thread_id
            || claim.message_id != message_id
            || claim.part != *part
            || claim.canceled
            || record.attachment.part != *part
        {
            return Err("staged attachment is bound to a different message or scope".into());
        }
        // Keep the durable claim as the monotonic staged->row transfer record.
        // Deleting it would let a retry/cancel race reclaim the same media.
    }
    Ok(())
}

pub(super) fn opaque_media(part: &ContentPart) -> Option<&MediaReference> {
    match part {
        ContentPart::Image { media, .. }
        | ContentPart::Audio { media, .. }
        | ContentPart::Video { media, .. }
        | ContentPart::File { media, .. } => Some(media),
        _ => None,
    }
}

pub(crate) fn validate_opaque_parts(parts: &[ContentPart]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::with_capacity(parts.len());
    for part in parts {
        let media = opaque_media(part)
            .ok_or_else(|| "stored attachment has an unsupported kind".to_string())?;
        let opaque_id = media
            .opaque_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "stored attachment must include an opaque reference".to_string())?;
        if media.path.is_some() || media.url.is_some() || media.local_cache_path.is_some() {
            return Err("stored attachment must not include a path or URL".into());
        }
        if !ids.insert(opaque_id) {
            return Err("stored attachment refs must be unique".into());
        }
        if let ContentPart::Video {
            thumbnail: Some(thumb),
            ..
        } = part
        {
            if thumb.path.is_some() || thumb.url.is_some() || thumb.local_cache_path.is_some() {
                return Err("stored attachment thumbnail must not include a path or URL".into());
            }
        }
    }
    Ok(())
}
