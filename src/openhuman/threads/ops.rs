//! RPC operations for conversation thread management.

use crate::openhuman::channels::providers::web as web_channel;
use crate::openhuman::config::Config;
use crate::openhuman::memory::conversations::{
    self, ConversationMessage, ConversationMessagePatch, ConversationThread,
    CreateConversationThread,
};
use crate::openhuman::memory::{
    ApiEnvelope, ApiMeta, AppendConversationMessageRequest, ConversationMessageRecord,
    ConversationMessagesRequest, ConversationMessagesResponse, ConversationThreadSummary,
    ConversationThreadsListResponse, DeleteConversationThreadRequest,
    DeleteConversationThreadResponse, EmptyRequest, GenerateConversationThreadTitleRequest,
    PaginationMeta, PurgeConversationThreadsResponse, UpdateConversationMessageRequest,
    UpsertConversationThreadRequest,
};
use crate::openhuman::providers::{self, ProviderRuntimeOptions};
use crate::openhuman::threads::title::{
    build_title_prompt, is_auto_generated_thread_title, sanitize_generated_title,
    title_log_fingerprint, THREAD_TITLE_LOG_PREFIX, THREAD_TITLE_MODEL_HINT,
    THREAD_TITLE_SYSTEM_PROMPT,
};
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

fn thread_to_summary(thread: ConversationThread) -> ConversationThreadSummary {
    ConversationThreadSummary {
        id: thread.id,
        title: thread.title,
        chat_id: thread.chat_id,
        is_active: thread.is_active,
        message_count: thread.message_count,
        last_message_at: thread.last_message_at,
        created_at: thread.created_at,
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

/// Lists all conversation threads.
pub async fn threads_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadsListResponse>>, String> {
    let dir = workspace_dir().await?;
    let threads = conversations::list_threads(dir)?
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
    let thread = conversations::ensure_thread(
        dir,
        CreateConversationThread {
            id: request.id,
            title: request.title,
            created_at: request.created_at,
        },
    )?;
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Creates a new conversation thread with auto-generated ID and title.
pub async fn thread_create_new(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now();
    let title = format!("Chat {} {}", now.format("%b %-d"), now.format("%-I:%M %p"));
    let created_at = chrono::Utc::now().to_rfc3339();
    let thread = conversations::ensure_thread(
        dir,
        CreateConversationThread {
            id,
            title,
            created_at,
        },
    )?;
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
    let messages = conversations::get_messages(dir, &request.thread_id)?
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

/// Appends a message to a conversation thread.
pub async fn message_append(
    request: AppendConversationMessageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    let dir = workspace_dir().await?;
    let message =
        conversations::append_message(dir, &request.thread_id, record_to_message(request.message))?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Generates a durable thread title from the first user message and assistant reply.
pub async fn thread_generate_title(
    request: GenerateConversationThreadTitleRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let dir = config.workspace_dir.clone();
    let Some(thread) = conversations::list_threads(dir.clone())?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(format!("thread {} not found", request.thread_id));
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

    let messages = conversations::get_messages(dir.clone(), &request.thread_id)?;
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
            "{THREAD_TITLE_LOG_PREFIX} no assistant message yet; skipping"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    let provider_runtime_options = ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };

    let provider = match providers::create_intelligent_routing_provider(
        config.api_url.as_deref(),
        &config,
        &provider_runtime_options,
    ) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} provider init failed; leaving placeholder title"
            );
            return Ok(envelope(
                thread_to_summary(thread),
                Some(counts([("num_threads", 1)])),
                None,
            ));
        }
    };

    tracing::debug!(
        thread_id = %request.thread_id,
        user_len = first_user_message.len(),
        assistant_len = assistant_message.len(),
        model = THREAD_TITLE_MODEL_HINT,
        "{THREAD_TITLE_LOG_PREFIX} generating thread title"
    );

    let raw_title = match provider
        .chat_with_system(
            Some(THREAD_TITLE_SYSTEM_PROMPT),
            &build_title_prompt(&first_user_message, &assistant_message),
            THREAD_TITLE_MODEL_HINT,
            0.2,
        )
        .await
    {
        Ok(title) => title,
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} title generation failed; leaving placeholder title"
            );
            return Ok(envelope(
                thread_to_summary(thread),
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
            "{THREAD_TITLE_LOG_PREFIX} generated empty title after sanitization"
        );
        return Ok(envelope(
            thread_to_summary(thread),
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

    let updated = conversations::update_thread_title(
        dir,
        &request.thread_id,
        &title,
        &chrono::Utc::now().to_rfc3339(),
    )?;

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

/// Updates metadata on an existing conversation message.
pub async fn message_update(
    request: UpdateConversationMessageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    let dir = workspace_dir().await?;
    let message = conversations::update_message(
        dir,
        &request.thread_id,
        &request.message_id,
        ConversationMessagePatch {
            extra_metadata: request.extra_metadata,
        },
    )?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Deletes a conversation thread and its message log.
pub async fn thread_delete(
    request: DeleteConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let dir = workspace_dir().await?;
    let deleted = conversations::ConversationStore::new(dir)
        .delete_thread(&request.thread_id, &request.deleted_at)?;
    web_channel::invalidate_thread_sessions(&request.thread_id).await;
    Ok(envelope(
        DeleteConversationThreadResponse { deleted },
        None,
        None,
    ))
}

/// Purges all conversation threads and messages.
pub async fn threads_purge(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    let stats = conversations::purge_threads(dir)?;
    Ok(envelope(
        PurgeConversationThreadsResponse {
            messages_deleted: stats.message_count,
            agent_threads_deleted: stats.thread_count,
            agent_messages_deleted: stats.message_count,
        },
        None,
        None,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
