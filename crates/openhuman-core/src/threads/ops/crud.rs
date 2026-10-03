//! Create, read, update, and delete operations for conversation threads and
//! their messages.

use super::support::{
    counts, envelope, message_to_record, record_to_message, run_to_completion, thread_to_summary,
    workspace_dir,
};
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
use crate::web_chat as web_channel;
use std::path::PathBuf;
use tinyagents_session::turn_state;

/// Lists all conversation threads.
pub async fn threads_list(
    _request: EmptyRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadsListResponse>>, String> {
    let dir = workspace_dir().await?;
    let threads = conversations::blocking::list_threads(dir)
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
    request: UpsertConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::ensure_thread(
        dir,
        conversations::CreateConversationThread {
            id: request.id,
            title: request.title,
            created_at: request.created_at,
            parent_thread_id: request.parent_thread_id,
            labels: request.labels,
            personality_id: request.personality_id,
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
    request: CreateConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now();
    let title = format!("Chat {} {}", now.format("%b %-d"), now.format("%-I:%M %p"));
    let created_at = chrono::Utc::now().to_rfc3339();
    let thread = conversations::blocking::ensure_thread(
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
        },
    )
    .await?;
    tracing::debug!(
        thread_id = %thread.id,
        labels = ?thread.labels,
        "[threads] created new thread"
    );
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Lists messages for a conversation thread.
pub async fn messages_list(
    request: ConversationMessagesRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessagesResponse>>, String> {
    let dir = workspace_dir().await?;
    let messages = conversations::blocking::get_messages(dir, request.thread_id.clone())
        .await?
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
    request: AppendConversationMessageRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
    let dir = workspace_dir().await?;
    let message = conversations::blocking::append_message(
        dir,
        request.thread_id.clone(),
        record_to_message(request.message),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Updates labels for a conversation thread.
///
/// An empty `labels` vec is valid and clears all labels from the thread,
/// making it invisible in every non-"All" filter view. Callers should
/// ensure this is intentional.
pub async fn thread_update_labels(
    request: UpdateConversationThreadLabelsRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::update_thread_labels(
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
    request: UpdateConversationThreadTitleRequest,
) -> Result<Outcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }
    let updated = conversations::blocking::update_thread_title(
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
    request: UpdateConversationMessageRequest,
) -> Result<Outcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    let dir = workspace_dir().await?;
    let message = conversations::blocking::update_message(
        dir,
        request.thread_id.clone(),
        request.message_id.clone(),
        ConversationMessagePatch {
            extra_metadata: request.extra_metadata,
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
    thread_id: &str,
    message_id: &str,
) -> Result<Option<usize>, ThreadsError> {
    let dir = workspace_dir().await?;
    conversations::blocking::delete_messages_from(
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
    request: DeleteConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("thread_delete", thread_delete_inner(dir, request)).await
}

async fn thread_delete_inner(
    dir: PathBuf,
    request: DeleteConversationThreadRequest,
) -> Result<Outcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let deleted = conversations::blocking::delete_thread(
        dir.clone(),
        request.thread_id.clone(),
        request.deleted_at.clone(),
    )
    .await?;
    // Invalidate the in-process web-channel session BEFORE the
    // turn-state cleanup. The snapshot deletion is fallible and
    // returns early on error; if invalidation ran after, an active
    // session for the now-deleted thread could linger and try to
    // append to a thread index row that no longer exists.
    web_channel::invalidate_thread_sessions(&request.thread_id).await;
    // Cancel any detached sub-agents this thread spawned BEFORE clearing their
    // queued results: abort the in-flight ones first so a child can't record a
    // completion in the gap between the two calls, then discard anything already
    // queued for delivery. Both target a thread that's being deleted, so there's
    // nowhere left to deliver to — abort + cleanup is the whole behavior.
    let cancelled =
        crate::agent::orchestration::running_subagents::cancel_for_thread(&request.thread_id);
    let discarded =
        crate::agent::orchestration::background_completions::discard_for_thread(&request.thread_id);
    log::debug!(
        "[threads] thread_delete thread_id={} cancelled_subagents={} discarded_completions={}",
        request.thread_id,
        cancelled,
        discarded
    );
    // Drop any persisted in-flight turn snapshot for this thread —
    // otherwise `threads_turn_state_list` keeps surfacing it (as
    // `Interrupted` on next restart) for a thread that no longer
    // exists. Failure here is surfaced as an RPC error so callers
    // can't observe a thread "deleted" while its snapshot (which
    // mirrors conversation-derived state) remains on disk; the
    // thread row itself is already gone at this point so the caller
    // sees a partial failure they can act on instead of silent drift.
    turn_state::store::delete(dir, &request.thread_id).map_err(|err| {
        format!(
            "thread {} deleted but turn-snapshot cleanup failed: {err}",
            request.thread_id
        )
    })?;
    Ok(envelope(
        DeleteConversationThreadResponse { deleted },
        None,
        None,
    ))
}
