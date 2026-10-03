//! Shared helpers used across the threads RPC operations: response envelope
//! construction, workspace resolution, cancellation-safe execution, and the
//! store-type <-> wire-type conversions every handler needs.

use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::threads::store as conversations;
use crate::threads::store::{ConversationMessage, ConversationThread};
use crate::threads::THREAD_TITLE_LOG_PREFIX;
use crate::threads::{ConversationMessageRecord, ConversationThreadSummary};
use std::path::PathBuf;
use tinyagents_harness::title::{title_from_user_message, title_log_fingerprint};

// One envelope/counts implementation for every ApiEnvelope-returning domain.
pub(super) use crate::core::envelope::{counts, envelope};

pub(super) async fn workspace_dir() -> Result<PathBuf, String> {
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
pub(super) async fn run_to_completion<T, F>(operation: &'static str, fut: F) -> Result<T, String>
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

pub(super) fn thread_to_summary(thread: ConversationThread) -> ConversationThreadSummary {
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

pub(super) fn message_to_record(message: ConversationMessage) -> ConversationMessageRecord {
    ConversationMessageRecord {
        id: message.id,
        content: message.content,
        message_type: message.message_type,
        extra_metadata: message.extra_metadata,
        sender: message.sender,
        created_at: message.created_at,
    }
}

pub(super) fn record_to_message(record: ConversationMessageRecord) -> ConversationMessage {
    ConversationMessage {
        id: record.id,
        content: record.content,
        message_type: record.message_type,
        extra_metadata: record.extra_metadata,
        sender: record.sender,
        created_at: record.created_at,
    }
}

pub(super) fn fallback_title_from_user_message(
    thread_id: &str,
    user_message: &str,
) -> Option<String> {
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

pub(super) async fn update_thread_with_fallback_title(
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
