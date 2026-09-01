use crate::core::runtime::context::CoreContext;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider;
use crate::openhuman::memory::{
    ApiEnvelope, ApiMeta, AppendConversationMessageRequest, ConversationMessageRecord,
    ConversationMessagesRequest, ConversationMessagesResponse, ConversationThreadSummary,
    ConversationThreadsListResponse, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, DeleteConversationThreadResponse, EmptyRequest,
    GenerateConversationThreadTitleRequest, PaginationMeta, PurgeConversationThreadsResponse,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};
// Every conversation-store call in this module goes through
// `conversations::blocking::*`, which runs the store's synchronous,
// globally-locked, fsync'ing operations on tokio's blocking pool. Calling the
// sync entry points directly from these handlers parked async worker threads on
// the store's `parking_lot` mutex, which starved the runtime and made
// `threads_create_new` blow the frontend's 30 s RPC budget (#5156).
use crate::openhuman::memory::conversations;
use crate::openhuman::memory::conversations::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
    CrossThreadHit,
};
use crate::openhuman::threads::title::{
    build_title_request, is_auto_generated_thread_title, sanitize_generated_title,
    title_from_user_message, title_log_fingerprint, THREAD_TITLE_LOG_PREFIX,
};
use crate::openhuman::threads::turn_state::{
    self, ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateForRequestRequest,
    GetTurnStateRequest, GetTurnStateResponse, ListTurnStatesResponse,
};
use crate::openhuman::threads::ThreadsError;
use crate::openhuman::web_chat as web_channel;
use crate::rpc::RpcOutcome;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn counts(entries: impl IntoIterator<Item = (&'static str, usize)>) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
    pagination: Option<PaginationMeta>,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination,
            },
        },
        vec![],
    )
}

async fn workspace_dir() -> Result<PathBuf, String> {
    Config::load_or_init()
        .await
        .map(|c| c.workspace_dir)
        .map_err(|e| format!("load config: {e}"))
}

/// Run a destructive sequence to completion even if the caller's future is
/// dropped (client disconnect, RPC timeout).
///
/// Moving the store onto the blocking pool (#5156) introduced a cancellation
/// point that did not exist before. `spawn_blocking` work is never cancelled
/// when its `JoinHandle` is dropped, so the store mutation lands regardless —
/// but the `.await` on that handle *is* a yield point, and previously the
/// synchronous store call had none. Dropping the handler there leaves the thread
/// deleted while the cleanup that follows it never runs: the web-channel session
/// stays live and can append to a thread index row that no longer exists,
/// detached sub-agents keep running and queueing completions, and the turn
/// snapshot survives to resurface as `Interrupted` for a thread that is gone.
/// Those are precisely the invariants `thread_delete`'s ordering comments exist
/// to hold.
///
/// Owning the mutation *and* its cleanup in one spawned task decouples the
/// sequence from the caller's lifetime. The ambient [`CoreContext`] is carried
/// across explicitly: a bare `tokio::spawn` drops the `task_local` scope, and
/// `CoreContext::current` then silently falls back to the process default —
/// which under multi-tenant scoped dispatch is the wrong workspace.
async fn run_to_completion<T, F>(operation: &'static str, fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let ctx = CoreContext::current();
    tokio::spawn(async move {
        match ctx {
            Some(ctx) => CoreContext::scope(ctx, fut).await,
            None => fut.await,
        }
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(
            operation,
            error = %error,
            "[threads] destructive task failed to join"
        );
        Err(format!("{operation} task failed: {error}"))
    })
}

fn thread_to_summary(thread: ConversationThread) -> ConversationThreadSummary {
    ConversationThreadSummary {
        id: thread.id,
        title: thread.title,
        chat_id: thread.chat_id,
        is_active: thread.is_active,
        message_count: thread.message_count,
        last_message_at: thread.last_message_at,
        created_at: thread.created_at,
        parent_thread_id: thread.parent_thread_id,
        labels: thread.labels,
        personality_id: thread.personality_id,
    }
}

fn message_to_record(message: ConversationMessage) -> ConversationMessageRecord {
    ConversationMessageRecord {
        id: message.id,
        content: message.content,
        message_type: message.message_type,
        extra_metadata: message.extra_metadata,
        sender: message.sender,
        created_at: message.created_at,
    }
}

fn record_to_message(record: ConversationMessageRecord) -> ConversationMessage {
    ConversationMessage {
        id: record.id,
        content: record.content,
        message_type: record.message_type,
        extra_metadata: record.extra_metadata,
        sender: record.sender,
        created_at: record.created_at,
    }
}

fn fallback_title_from_user_message(thread_id: &str, user_message: &str) -> Option<String> {
    let title = title_from_user_message(user_message);
    if let Some(title) = &title {
        tracing::debug!(
            thread_id = %thread_id,
            title_len = title.chars().count(),
            title_hash = %title_log_fingerprint(title),
            "{THREAD_TITLE_LOG_PREFIX} derived fallback title from user message"
        );
    } else {
        tracing::debug!(
            thread_id = %thread_id,
            "{THREAD_TITLE_LOG_PREFIX} user message did not yield fallback title"
        );
    }
    title
}

async fn update_thread_with_fallback_title(
    dir: PathBuf,
    thread: ConversationThread,
    user_message: &str,
) -> Result<ConversationThread, String> {
    let Some(title) = fallback_title_from_user_message(&thread.id, user_message) else {
        return Ok(thread);
    };
    if title == thread.title {
        return Ok(thread);
    }
    conversations::blocking::update_thread_title(
        dir,
        thread.id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
}

/// Lists all conversation threads.
pub async fn threads_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadsListResponse>>, String> {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::ensure_thread(
        dir,
        CreateConversationThread {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now();
    let title = format!("Chat {} {}", now.format("%b %-d"), now.format("%-I:%M %p"));
    let created_at = chrono::Utc::now().to_rfc3339();
    let thread = conversations::blocking::ensure_thread(
        dir,
        CreateConversationThread {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessagesResponse>>, String> {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
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

/// Generates a durable thread title from the first user message and assistant reply.
pub async fn thread_generate_title(
    request: GenerateConversationThreadTitleRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, ThreadsError> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let dir = config.workspace_dir.clone();
    let Some(thread) = conversations::blocking::list_threads(dir.clone())
        .await?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(ThreadsError::not_found(request.thread_id));
    };

    if !is_auto_generated_thread_title(&thread.title) {
        tracing::debug!(
            thread_id = %request.thread_id,
            title_len = thread.title.chars().count(),
            title_hash = %title_log_fingerprint(&thread.title),
            "{THREAD_TITLE_LOG_PREFIX} skipping non-placeholder title"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let messages =
        conversations::blocking::get_messages(dir.clone(), request.thread_id.clone()).await?;
    let Some(first_user_message) = messages
        .iter()
        .find(|message| message.sender == "user" && !message.content.trim().is_empty())
        .map(|message| message.content.trim().to_string())
    else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no user message yet; skipping"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    let assistant_message = request
        .assistant_message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            messages
                .iter()
                .find(|message| message.sender == "agent" && !message.content.trim().is_empty())
                .map(|message| message.content.trim().to_string())
        });

    let Some(assistant_message) = assistant_message else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no assistant message yet; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    // `_with_model_id` rather than the plain constructor: the debug line below
    // reports the model this call actually dispatches on, and only the factory
    // knows what the `summarization` role resolved to for this configuration.
    let (chat_model, resolved_model) =
        match provider::create_chat_model_with_model_id("summarization", &config, 0.2) {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::warn!(
                    thread_id = %request.thread_id,
                    error = %error,
                    "{THREAD_TITLE_LOG_PREFIX} provider init failed; applying fallback title"
                );
                let updated =
                    update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
                return Ok(envelope(
                    thread_to_summary(updated),
                    Some(counts([("num_threads", 1)])),
                    None,
                ));
            }
        };

    tracing::debug!(
        thread_id = %request.thread_id,
        user_len = first_user_message.len(),
        assistant_len = assistant_message.len(),
        model = %resolved_model,
        "{THREAD_TITLE_LOG_PREFIX} generating thread title"
    );

    let raw_title = match chat_model
        .invoke(
            &(),
            build_title_request(&first_user_message, &assistant_message),
        )
        .await
    {
        Ok(response) => response.text(),
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} title generation failed; applying fallback title"
            );
            let updated =
                update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
            return Ok(envelope(
                thread_to_summary(updated),
                Some(counts([("num_threads", 1)])),
                None,
            ));
        }
    };

    let Some(title) = sanitize_generated_title(&raw_title) else {
        tracing::warn!(
            thread_id = %request.thread_id,
            raw_title_len = raw_title.chars().count(),
            raw_title_hash = %title_log_fingerprint(&raw_title),
            "{THREAD_TITLE_LOG_PREFIX} generated empty title after sanitization; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    if title == thread.title {
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let updated = conversations::blocking::update_thread_title(
        dir,
        request.thread_id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;

    tracing::debug!(
        thread_id = %request.thread_id,
        title_len = updated.title.chars().count(),
        title_hash = %title_log_fingerprint(&updated.title),
        "{THREAD_TITLE_LOG_PREFIX} updated thread title"
    );

    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
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
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, String> {
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

/// Deletes a conversation thread and its message log.
///
/// The store mutation and every cleanup step it implies run inside one
/// [`run_to_completion`] task, so a caller that disconnects mid-delete cannot
/// leave the thread gone from the store with its sessions, sub-agents and turn
/// snapshot still live.
pub async fn thread_delete(
    request: DeleteConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("thread_delete", thread_delete_inner(dir, request)).await
}

async fn thread_delete_inner(
    dir: PathBuf,
    request: DeleteConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
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
    let cancelled = crate::openhuman::agent::orchestration::running_subagents::cancel_for_thread(
        &request.thread_id,
    );
    let discarded =
        crate::openhuman::agent::orchestration::background_completions::discard_for_thread(
            &request.thread_id,
        );
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

/// Purges all conversation threads and messages.
///
/// Same cancellation contract as [`thread_delete`]: the purge and its sub-agent
/// / turn-snapshot cleanup are one [`run_to_completion`] unit, so a dropped
/// caller cannot leave every thread wiped while their sub-agents keep running.
pub async fn threads_purge(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("threads_purge", threads_purge_inner(dir)).await
}
