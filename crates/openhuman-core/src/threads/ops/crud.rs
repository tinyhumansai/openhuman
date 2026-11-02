//! Create, read, update, and delete operations for conversation threads and
//! their messages.

use super::support::{
    counts, envelope, message_to_record, record_to_message, run_to_completion, thread_to_summary,
    workspace_dir,
};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::store as conversations;
use crate::threads::store::{ConversationMessagePatch, CrossThreadHit};
use crate::threads::ThreadsError;
use crate::threads::{
    ApiEnvelope, AppendConversationMessageRequest, ConversationMessageRecord,
    ConversationMessagesRequest, ConversationMessagesResponse, ConversationThreadSummary,
    ConversationThreadsListResponse, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, DeleteConversationThreadResponse, EmptyRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};

/// Lists all conversation threads.
pub async fn threads_list(
    ctx: &DispatchCtx,
    _request: EmptyRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadsListResponse>>, String> {
    let dir = workspace_dir().await?;
    let threads = conversations::blocking::list_threads(ctx.clone(), dir)
        .await?
        .into_iter()
        .map(thread_to_summary)
        .collect::<Vec<_>>();
    let count = threads.len();
    Ok(envelope(
        ConversationThreadsListResponse { threads, count },
        Some(counts([("num_threads", count)])),
        None,
    ))
}

/// Creates or refreshes a conversation thread.
pub async fn thread_upsert(
    ctx: &DispatchCtx,
    request: UpsertConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::ensure_thread(
        ctx.clone(),
        dir,
        conversations::CreateConversationThread {
            id: request.id,
            title: request.title,
            created_at: request.created_at,
            parent_thread_id: request.parent_thread_id,
            labels: request.labels,
            personality_id: request.personality_id,
            working_dir: None,
        },
    )
    .await?;
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Creates a new conversation thread with auto-generated ID and title.
pub async fn thread_create_new(
    ctx: &DispatchCtx,
    request: CreateConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let working_dir =
        super::working_dir::validate_working_dir(request.action_dir.as_deref()).await?;
    let id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now();
    let title = format!("Chat {} {}", now.format("%b %-d"), now.format("%-I:%M %p"));
    let created_at = chrono::Utc::now().to_rfc3339();
    let thread = conversations::blocking::ensure_thread(
        ctx.clone(),
        dir,
        conversations::CreateConversationThread {
            id,
            title,
            created_at,
            parent_thread_id: None,
            // Pass labels through as-is; the store's infer_labels() applies
            // the same default on index rebuild, so this is the single source
            // of truth for default labels.
            labels: request.labels,
            personality_id: request.personality_id,
            working_dir,
        },
    )
    .await?;
    tracing::debug!(
        thread_id = %thread.id,
        labels = ?thread.labels,
        working_dir_bound = thread.working_dir.is_some(),
        "[threads] created new thread"
    );
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Lists messages for a conversation thread, artifact rows included: the UI
/// renders each artifact's card from its row (see
/// [`crate::threads::artifact_rows`]). Artifacts that have no row yet get one
/// first, so a thread's older artifacts are placed too.
pub async fn messages_list(
    ctx: &DispatchCtx,
    request: ConversationMessagesRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessagesResponse>>, String> {
    let dir = workspace_dir().await?;
    let mut messages = conversations::blocking::get_messages_with_artifact_rows(
        ctx.clone(),
        dir.clone(),
        &request.thread_id,
    )
    .await?;
    match crate::threads::artifact_rows::backfill_artifact_rows(
        ctx,
        &dir,
        &request.thread_id,
        &messages,
    )
    .await
    {
        Ok(written) => messages.extend(written),
        // The list still serves; the rows are retried on the next read.
        Err(error) => log::warn!(
            "[threads] artifact row backfill failed thread_id={}: {error}",
            request.thread_id
        ),
    }
    let messages = messages
        .into_iter()
        .map(message_to_record)
        .collect::<Vec<_>>();
    let count = messages.len();
    Ok(envelope(
        ConversationMessagesResponse { messages, count },
        Some(counts([("num_messages", count)])),
        None,
    ))
}

/// Search messages across **every** thread in the workspace for a query,
/// returning up to `limit` of the most-recent matches (newest first). Backed
/// by the trigram/CJK-bigram inverted index in `memory_conversations` — the
/// same cross-chat reader the durable-context pipeline uses (issue #1505).
///
/// Read-only and workspace-scoped. `exclude_thread_id` lets a caller drop the
/// active chat from the results when it already has that context in hand.
pub async fn transcript_search(
    ctx: &DispatchCtx,
    query: &str,
    limit: usize,
    exclude_thread_id: Option<&str>,
) -> Result<Vec<CrossThreadHit>, String> {
    let dir = workspace_dir().await?;
    log::debug!(
        "[threads][transcript_search] query_chars={} limit={} exclude={:?}",
        query.chars().count(),
        limit,
        exclude_thread_id
    );
    let hits = conversations::blocking::search_cross_thread_messages(
        ctx.clone(),
        dir,
        query.to_string(),
        limit,
        exclude_thread_id.map(str::to_string),
    )
    .await?;
    log::debug!("[threads][transcript_search] hits={}", hits.len());
    Ok(hits)
}

/// Appends a message to a conversation thread.
pub async fn message_append(
    ctx: &DispatchCtx,
    request: AppendConversationMessageRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
    let origin = ctx.turn_origin_or_scoped();
    let config = ctx
        .load_config()
        .await
        .map_err(|error| ThreadsError::Message(format!("load config: {error}")))?;
    let persistence_dir = config.workspace_dir.clone();
    message_append_with_config(ctx, request, &config, origin.as_ref(), &persistence_dir).await
}

pub(super) async fn message_append_with_config(
    ctx: &DispatchCtx,
    mut request: AppendConversationMessageRequest,
    config: &crate::config::Config,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
    persistence_dir: &std::path::Path,
) -> Result<Outcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
    // Reserve the private path map for values minted after core staging. A
    // caller may send arbitrary public metadata but never storage authority.
    crate::threads::attachments::strip_private_metadata(&mut request.message.extra_metadata);
    let typed_attachments = if request.message.sender == "user" {
        std::mem::take(&mut request.attachments)
    } else {
        Vec::new()
    };
    let typed_media_count = typed_attachments
        .iter()
        .filter(|part| {
            matches!(
                part,
                tinychannels_bus::ContentPart::Image { .. }
                    | tinychannels_bus::ContentPart::Audio { .. }
                    | tinychannels_bus::ContentPart::Video { .. }
                    | tinychannels_bus::ContentPart::File { .. }
            )
        })
        .count();
    let staged_refs = if !typed_attachments.is_empty()
        && crate::threads::attachments::validate_opaque_parts(&typed_attachments).is_ok()
    {
        request.message.attachments = crate::threads::attachments::bind_staged_refs(
            ctx,
            &request.thread_id,
            config,
            &typed_attachments,
            &request.message.id,
            &mut request.message.extra_metadata,
        )
        .await
        .map_err(ThreadsError::Message)?;
        Some(typed_attachments.clone())
    } else {
        None
    };
    if typed_attachments
        .iter()
        .any(|part| matches!(part, tinychannels_bus::ContentPart::VoiceNote { .. }))
    {
        return Err(ThreadsError::Message(
            "voice notes must be sent through channel.web_chat".into(),
        ));
    }
    let original_content = request.message.content.clone();
    if !typed_attachments.is_empty()
        && (original_content.contains("[IMAGE:")
            || original_content.contains("[FILE:")
            || !crate::attachments::parse(&original_content).1.is_empty())
    {
        return Err(ThreadsError::Message(
            "typed attachments cannot be combined with legacy attachment markers".into(),
        ));
    }
    if !typed_attachments.is_empty() && staged_refs.is_none() {
        request.message.content = crate::attachments::parts::fold_into_message(
            &request.message.content,
            &typed_attachments,
        )
        .map_err(|error| ThreadsError::Message(error.to_string()))?;
    }
    if request.message.sender == "user" {
        let raw_upload = request.message.content.contains("[IMAGE:")
            || request.message.content.contains("[FILE:");
        let durable_files = crate::attachments::parse(&request.message.content).1;
        let durable_upload = !durable_files.is_empty();
        // Raw and durable uploads both live under the thread's working folder,
        // where the turn reads them (#7714).
        let workspace = if raw_upload || durable_upload {
            crate::attachments::thread_action_dir(
                ctx,
                config,
                persistence_dir,
                Some(&request.thread_id),
            )
            .await
            .map_err(|error| ThreadsError::Message(format!("stage user attachments: {error}")))?
        } else {
            config.action_dir.clone()
        };
        if durable_upload {
            let image_count = durable_files
                .iter()
                .filter(|file| file.mime.starts_with("image/"))
                .count();
            let (without_images, raw_images) =
                tinyagents_harness::multimodal::markers::parse_image_markers(
                    &request.message.content,
                );
            let (_, raw_files) =
                tinyagents_harness::multimodal::markers::parse_file_markers(&without_images);
            let (max_images, image_mb) = config.multimodal.effective_limits();
            let (max_files, file_mb, _) = config.multimodal_files.effective_limits();
            if config.multimodal_files.max_files == 0
                || image_count + raw_images.len() > max_images
                || durable_files.len() - image_count + raw_files.len() > max_files
                || (image_count > 0 && config.multimodal.max_images == 0)
                || matches!(
                    origin,
                    Some(
                        crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. }
                    )
                )
            {
                return Err(ThreadsError::Message(
                    "attachment count exceeds configured limit".into(),
                ));
            }
            for file in &durable_files {
                let scope = crate::attachments::AttachmentAccessScope {
                    external_channel: matches!(
                        origin,
                        Some(
                            crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. }
                        )
                    ),
                    workspace: Some(workspace.clone()),
                    ..Default::default()
                };
                let path = crate::attachments::resolve_path(config, &file.path, &scope)
                    .await
                    .map_err(|error| {
                        ThreadsError::Message(format!("validate user attachment: {error}"))
                    })?;
                let metadata = tokio::fs::metadata(path).await.map_err(|error| {
                    ThreadsError::Message(format!("validate user attachment: {error}"))
                })?;
                let cap = if file.mime.starts_with("image/") {
                    image_mb
                } else {
                    file_mb
                } * 1024
                    * 1024;
                if !metadata.is_file() || metadata.len() > cap as u64 {
                    return Err(ThreadsError::Message(
                        "attachment exceeds configured file limit".into(),
                    ));
                }
            }
        }
        let byte_fields = [
            "attachmentDataUris",
            "attachmentData",
            "attachmentDataUri",
            "attachmentPosters",
            "attachmentPoster",
            "attachmentPreviews",
            "attachmentPreview",
        ];
        let has_upload_bytes = byte_fields.iter().any(|field| {
            request
                .message
                .extra_metadata
                .get(*field)
                .is_some_and(|value| !value.is_null() && value != &serde_json::json!([]))
        });
        if has_upload_bytes && !raw_upload && !durable_upload {
            return Err(ThreadsError::Message(
                "user upload bytes require attachment markers before persistence".into(),
            ));
        }
        let staged_content = crate::attachments::stage_turn(
            &request.message.content,
            Some(config),
            Some(&tinytools::WorkspaceDescriptor::new(workspace)),
            Some(&request.thread_id),
            origin,
        )
        .await
        .map_err(|error| ThreadsError::Message(format!("stage user attachments: {error}")))?;
        if typed_media_count > 0 && staged_refs.is_none() {
            let (clean_content, staged_files) = crate::attachments::parse(&staged_content);
            if staged_files.len() != typed_media_count {
                return Err(ThreadsError::Message(
                    "typed attachment staging did not preserve media order".into(),
                ));
            }
            let entries =
                crate::threads::attachments::from_staged_parts(&typed_attachments, &staged_files)
                    .map_err(ThreadsError::Message)?;
            request.message.attachments =
                crate::threads::attachments::save(&mut request.message.extra_metadata, entries);
            request.message.content = clean_content.trim().to_string();
        } else {
            request.message.content = staged_content;
        }
        if raw_upload || durable_upload {
            if let Some(metadata) = request.message.extra_metadata.as_object_mut() {
                for field in byte_fields {
                    metadata.remove(field);
                }
            }
        }
    }
    let expected_id = request.message.id.clone();
    let expected_sender = request.message.sender.clone();
    let expected_content = request.message.content.clone();
    let expected_attachments = request.message.attachments.clone();
    let message = conversations::blocking::append_message(
        ctx.clone(),
        persistence_dir.to_path_buf(),
        request.thread_id.clone(),
        record_to_message(request.message),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;
    let message = message_to_record(message);
    if message.id != expected_id
        || message.sender != expected_sender
        || message.content != expected_content
        || message.attachments != expected_attachments
    {
        if let Some(staged_refs) = &staged_refs {
            let _ = crate::threads::attachments::discard_staged_parts(
                ctx,
                &request.thread_id,
                staged_refs,
            )
            .await;
        }
        return Err(ThreadsError::Message(
            "message ID conflicts with an existing persisted row".into(),
        ));
    }
    if let Some(staged_refs) = staged_refs {
        crate::threads::attachments::consume_staged_parts(
            ctx,
            &request.thread_id,
            &staged_refs,
            &message.id,
        )
        .await
        .map_err(ThreadsError::Message)?;
    }
    Ok(envelope(message, Some(counts([("num_messages", 1)])), None))
}

#[cfg(test)]
#[path = "crud_message_append_tests.rs"]
mod message_append_tests;

#[cfg(test)]
#[path = "crud_queued_attachment_tests.rs"]
mod queued_attachment_tests;

/// Updates labels for a conversation thread.
///
/// An empty `labels` vec is valid and clears all labels from the thread,
/// making it invisible in every non-"All" filter view. Callers should
/// ensure this is intentional.
pub async fn thread_update_labels(
    ctx: &DispatchCtx,
    request: UpdateConversationThreadLabelsRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::update_thread_labels(
        ctx.clone(),
        dir,
        request.thread_id.clone(),
        request.labels.clone(),
        chrono::Utc::now().to_rfc3339(),
    )
    .await?;
    tracing::debug!(
        thread_id = %request.thread_id,
        labels = ?request.labels,
        "[threads] updated thread labels"
    );
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Sets a user-specified title on a conversation thread, bypassing AI generation.
pub async fn thread_update_title(
    ctx: &DispatchCtx,
    request: UpdateConversationThreadTitleRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }
    let updated = conversations::blocking::update_thread_title(
        ctx.clone(),
        dir,
        request.thread_id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| format!("update title: {err}"))?;
    tracing::debug!(
        thread_id = %request.thread_id,
        title_len = updated.title.chars().count(),
        "[threads] user updated thread title"
    );
    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Updates metadata on an existing conversation message.
pub async fn message_update(
    ctx: &DispatchCtx,
    request: UpdateConversationMessageRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    if request
        .extra_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("__openhumanStoredAttachments"))
        .is_some()
    {
        return Err("private attachment storage metadata is reserved".into());
    }
    let dir = ctx
        .load_config()
        .await
        .map_err(|error| format!("load config: {error}"))?
        .workspace_dir;
    let message = conversations::blocking::update_message(
        ctx.clone(),
        dir,
        request.thread_id.clone(),
        request.message_id.clone(),
        ConversationMessagePatch {
            // Merge public metadata inside the store's CAS retry. This keeps
            // the private attachment path map invisible to callers while
            // preventing an ordinary UI metadata update from erasing it.
            merge_extra_metadata: request.extra_metadata,
            ..Default::default()
        },
    )
    .await?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Truncates a thread's message log at `message_id`: removes that message and
/// everything appended after it, keeping everything before it. Backs
/// `threads.edit_message` / `threads.regenerate` (`web_chat::ops::edit`),
/// which cut the message log's tail before restarting the turn from an
/// earlier point.
///
/// `Ok(None)` means `message_id` was not found in the thread — the caller
/// should treat that as "nothing to truncate" (e.g. a stale/already-edited
/// message id), not as an empty thread.
pub async fn delete_after(
    ctx: &DispatchCtx,
    thread_id: &str,
    message_id: &str,
) -> Result<Option<usize>, ThreadsError> {
    let dir = workspace_dir().await?;
    conversations::blocking::delete_messages_from(
        ctx.clone(),
        dir,
        thread_id.to_string(),
        message_id.to_string(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(thread_id, err))
}

/// Deletes a conversation thread and its message log.
///
/// The store mutation and every cleanup step it implies run inside one
/// [`run_to_completion`] task, so a caller that disconnects mid-delete cannot
/// leave the thread gone from the store with its sessions, sub-agents and turn
/// snapshot still live.
pub async fn thread_delete(
    ctx: &DispatchCtx,
    request: DeleteConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    run_to_completion(
        ctx,
        "thread_delete",
        super::thread_delete::thread_delete_inner(ctx.clone(), config, request),
    )
    .await
}
