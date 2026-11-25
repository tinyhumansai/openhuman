//! Durable typed attachment references for conversation messages.
//!
//! The public message stores opaque TinyChannels media parts separately from
//! its text. Workspace-relative file paths remain private to OpenHuman's
//! persisted metadata and are resolved only under the trusted dispatch scope
//! when a turn is about to execute.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tinychannels_bus::{ContentPart, MediaReference};
use tinystoragedrivers::Precondition;

use crate::attachments::Attachment;
use crate::storage::{self, documents::Repo};

const STORAGE_KEY: &str = "__openhumanStoredAttachments";

#[path = "attachments/staged.rs"]
mod staged;
use staged::*;
pub(crate) use staged::{
    bind_staged_refs, commit_native_channel_parts, consume_staged_parts, discard_staged_parts,
    native_parts_in_committed_message, read_authorized_native_attachment,
    stage_native_channel_part, stage_queued_parts, validate_opaque_parts, NativeStagedLease,
};

pub(crate) fn has_opaque_ref(part: &ContentPart) -> bool {
    opaque_media(part).is_some_and(|media| media.opaque_id.is_some())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAttachment {
    part: ContentPart,
    path: String,
}

/// Persist sanitized typed parts and their private staged paths in message
/// metadata. The public row projection strips the private path records.
pub(crate) fn save(metadata: &mut Value, entries: Vec<(ContentPart, String)>) -> Vec<ContentPart> {
    let stored = entries
        .into_iter()
        .map(|(part, path)| StoredAttachment { part, path })
        .collect::<Vec<_>>();
    let public = stored.iter().map(|entry| entry.part.clone()).collect();
    if !metadata.is_object() {
        *metadata = json!({});
    }
    metadata[STORAGE_KEY] = serde_json::to_value(stored).expect("stored attachments serialize");
    public
}

/// Public opaque refs recorded on a message row, with no local path or URL.
pub(crate) fn public_refs(metadata: &Value) -> Vec<ContentPart> {
    stored(metadata)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.part)
        .collect()
}

/// Remove the private path map from metadata returned over RPC.
pub(crate) fn strip_private_metadata(metadata: &mut Value) {
    if let Some(object) = metadata.as_object_mut() {
        object.remove(STORAGE_KEY);
    }
}

/// Pair typed media input with the staged files created from it, storing only
/// opaque refs publicly and keeping each validated relative path privately.
pub(crate) fn from_staged_parts(
    parts: &[ContentPart],
    files: &[Attachment],
) -> Result<Vec<(ContentPart, String)>, String> {
    let media_count = parts
        .iter()
        .filter(|part| {
            matches!(
                part,
                ContentPart::Image { .. }
                    | ContentPart::Audio { .. }
                    | ContentPart::Video { .. }
                    | ContentPart::File { .. }
            )
        })
        .count();
    if media_count != files.len() {
        return Err("typed attachment staging did not preserve media ordering".into());
    }

    let mut files = files.iter();
    let mut output = Vec::with_capacity(media_count);
    for part in parts {
        match part {
            ContentPart::Image {
                media,
                caption,
                alt_text,
            } => {
                let file = files.next().expect("media count was checked");
                output.push((
                    ContentPart::Image {
                        media: stored_media(media, file),
                        caption: caption.clone(),
                        alt_text: alt_text.clone(),
                    },
                    file.path.clone(),
                ));
            }
            ContentPart::Audio {
                media,
                codec,
                duration_ms,
                title,
                transcript,
            } => {
                let file = files.next().expect("media count was checked");
                output.push((
                    ContentPart::Audio {
                        media: stored_media(media, file),
                        codec: codec.clone(),
                        duration_ms: *duration_ms,
                        title: title.clone(),
                        transcript: transcript.clone(),
                    },
                    file.path.clone(),
                ));
            }
            ContentPart::Video {
                media,
                caption,
                duration_ms,
                thumbnail,
            } => {
                let file = files.next().expect("media count was checked");
                output.push((
                    ContentPart::Video {
                        media: stored_media(media, file),
                        caption: caption.clone(),
                        duration_ms: *duration_ms,
                        thumbnail: thumbnail.as_ref().map(|thumb| {
                            let mut sanitized = thumb.clone();
                            sanitized.path = None;
                            sanitized.url = None;
                            sanitized.local_cache_path = None;
                            sanitized.opaque_id = None;
                            sanitized
                        }),
                    },
                    file.path.clone(),
                ));
            }
            ContentPart::File {
                media,
                file_name,
                caption,
            } => {
                let file = files.next().expect("media count was checked");
                let mut stored = stored_media(media, file);
                if let Some(name) = file_name {
                    stored.file_name = Some(name.clone());
                }
                output.push((
                    ContentPart::File {
                        media: stored,
                        file_name: file_name.clone(),
                        caption: caption.clone(),
                    },
                    file.path.clone(),
                ));
            }
            ContentPart::Text { .. } | ContentPart::VoiceNote { .. } => {}
            _ => return Err("typed attachment has an unsupported kind".into()),
        }
    }
    Ok(output)
}

fn stored_media(input: &MediaReference, file: &Attachment) -> MediaReference {
    let mut media = input.clone();
    media.opaque_id = Some(uuid::Uuid::new_v4().simple().to_string());
    media.path = None;
    media.url = None;
    media.local_cache_path = None;
    media.content_type = Some(file.mime.clone());
    media.file_name = input.file_name.clone().or_else(|| Some(file.name.clone()));
    media.size_bytes = Some(file.size_bytes as u64);
    media
}

fn stored(metadata: &Value) -> Result<Vec<StoredAttachment>, String> {
    let Some(value) = metadata.get(STORAGE_KEY) else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value.clone())
        .map_err(|error| format!("stored attachment metadata is invalid: {error}"))
}

fn tenant_for(
    ctx: &crate::core::runtime::DispatchCtx,
) -> Result<crate::core::runtime::Tenant, String> {
    crate::core::runtime::tenant::tenant_in(
        crate::core::runtime::is_saas(),
        ctx.resolved().as_deref(),
    )
    .map_err(|error| error.to_string())
}

/// Resolve request refs against one persisted user row. The request may carry
/// opaque IDs and descriptive metadata only; storage paths come exclusively
/// from the exact row's private metadata.
pub(crate) async fn resolve_persisted_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    persisted_message_id: &str,
    requested: &[ContentPart],
) -> Result<Vec<ContentPart>, String> {
    if thread_id.is_empty() || persisted_message_id.is_empty() || requested.is_empty() {
        return Err("stored attachment scope and refs must be non-empty".into());
    }
    validate_opaque_parts(requested)?;

    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("stored attachment configuration unavailable: {error}"))?;
    let messages =
        crate::threads::store::get_messages(ctx, config.workspace_dir.clone(), thread_id)?;
    let row = messages
        .iter()
        .find(|message| message.id == persisted_message_id && message.sender == "user")
        .ok_or_else(|| "stored attachment message is missing or is not a user row".to_string())?;
    let entries = stored(&row.extra_metadata)?;
    let stored_parts = entries
        .iter()
        .map(|entry| entry.part.clone())
        .collect::<Vec<_>>();
    if requested != stored_parts {
        return Err("stored attachment refs do not match the persisted user row".into());
    }

    validate_staged_files(ctx, thread_id, &config, &entries)
        .await
        .map_err(|error| error.replace("staged", "stored"))?;

    let mut resolved = Vec::with_capacity(entries.len());
    for mut entry in entries {
        let media = opaque_media_mut(&mut entry.part)
            .ok_or_else(|| "stored attachment has an unsupported kind".to_string())?;
        media.path = Some(entry.path);
        resolved.push(entry.part);
    }
    Ok(resolved)
}

/// Re-stage media already owned by the exact source row for an edit.
///
/// The request supplies only opaque refs. Paths are recovered from the
/// profile/thread-scoped persisted row, validated under the trusted context,
/// then copied into fresh staging records so the replacement row receives
/// fresh opaque IDs and does not inherit the deleted row's claim.
pub(crate) async fn restage_persisted_parts(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    persisted_message_id: &str,
    requested: &[ContentPart],
) -> Result<Vec<ContentPart>, String> {
    use base64::Engine as _;

    let mut resolved =
        resolve_persisted_parts(ctx, thread_id, persisted_message_id, requested).await?;
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("stored attachment configuration unavailable: {error}"))?;
    let workspace =
        crate::attachments::thread_action_dir(ctx, &config, &config.workspace_dir, Some(thread_id))
            .await
            .map_err(|error| format!("stored attachment workspace unavailable: {error}"))?;
    let scope = crate::attachments::AttachmentAccessScope {
        external_channel: false,
        workspace: Some(workspace),
        ..Default::default()
    };
    for part in &mut resolved {
        let media = opaque_media_mut(part)
            .ok_or_else(|| "stored attachment has an unsupported kind".to_string())?;
        let relative_path = media
            .path
            .take()
            .ok_or_else(|| "stored attachment path is unavailable".to_string())?;
        let mime = media
            .content_type
            .as_deref()
            .filter(|mime| {
                !mime.is_empty() && !mime.chars().any(|ch| matches!(ch, '\r' | '\n' | ']'))
            })
            .ok_or_else(|| "stored attachment content type is unavailable".to_string())?;
        let path = crate::attachments::resolve_path(&config, &relative_path, &scope)
            .await
            .map_err(|error| format!("resolve validated stored attachment: {error}"))?;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("read validated stored attachment: {error}"))?;
        media.opaque_id = None;
        media.url = Some(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ));
    }
    stage_queued_parts(ctx, thread_id, &resolved).await
}

fn opaque_media_mut(part: &mut ContentPart) -> Option<&mut MediaReference> {
    match part {
        ContentPart::Image { media, .. }
        | ContentPart::Audio { media, .. }
        | ContentPart::Video { media, .. }
        | ContentPart::File { media, .. } => Some(media),
        _ => None,
    }
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
