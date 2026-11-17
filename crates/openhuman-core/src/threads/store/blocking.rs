//! Async wrappers that run the conversation store's **blocking** operations on
//! tokio's blocking pool (#5156).
//!
//! # Why this lives here (#5560)
//!
//! It was `tinymemory_core::conversations::blocking`, and that crate's own
//! module docs already called it *host-retained*: the conversation store is the
//! engine's, but deciding which executor a host runs a blocking call on is the
//! host's, and nothing inside `tinymemory` ever called these wrappers. So this
//! is a move home rather than a re-routing — the same shape as
//! `memory::rpc_models`, whose forty-five types were named only by this host.
//!
//! The store these wrappers address has since followed them home: `store` is
//! now [`super`]'s own subtree rather than the retired v1 memory engine's
//! `conversations` module, and the import below is the only line that changed for it. The item set is
//! the same one the engine exported, so function signatures, argument order,
//! error strings and the `[conversations]` log prefix stay byte-identical —
//! `web_chat::turn_host` and the RPC layer read them.
//!
//! Every store entry point is synchronous and does fsync'd JSONL file IO. The
//! store now coordinates lifecycle per conversation root, shared metadata per
//! root, and message files per thread, so unrelated agents no longer queue
//! behind one process-global mutex. Calling it directly from an `async fn`
//! can still park a tokio **worker** thread for disk latency or a contended
//! per-root lock, and the wait is not always short:
//!
//! * `threads.jsonl` is folded from scratch on nearly every operation
//!   (`thread_index_unlocked`), and it grows by roughly two lines per appended
//!   message and is never compacted — so the per-call fold cost grows with the
//!   user's whole history;
//! * `append_message` writes a message under its per-thread lock and serializes
//!   the compact metadata append; `update_message` reads and rewrites an entire
//!   thread log under that thread's lock;
//! * `search_cross_thread_messages` reads every thread's transcript on a cold
//!   index.
//!
//! Historically, once more concurrent conversation operations than there were
//! worker threads parked on the global mutex, the runtime stopped polling
//! **anything** — including the HTTP task that owed the client its response.
//! That is how a create that
//! only needs one append blows the frontend's 30 s RPC budget:
//! `UnhandledRejection: Core RPC openhuman.threads_create_new timed out after
//! 30000ms` (Sentry TAURI-REACT-10, #5156).
//!
//! Moving each call onto the blocking pool keeps disk and lock waits off the
//! async workers. Sharded store locks additionally let independent agent
//! threads perform their transcript IO concurrently; only their short shared
//! metadata appends remain serialized per conversation root.
//!
//! Callers pass owned arguments because the closure must be `'static`.

use std::path::PathBuf;

use crate::core::runtime::DispatchCtx;
use crate::threads::store;

use super::{
    ConversationMessage, ConversationMessagePatch, ConversationPurgeStats, ConversationStore,
    ConversationThread, CreateConversationThread, CrossThreadHit,
};

/// Run one blocking store call on the blocking pool.
///
/// A `JoinError` here means the pool task panicked (or the runtime is shutting
/// down); surface it as a store error rather than propagating the panic into
/// the RPC dispatcher, which would turn a transient store fault into a 500-shaped
/// unknown failure.
async fn run<T, F>(ctx: DispatchCtx, operation: &'static str, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match ctx.spawn_blocking(f).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                operation,
                error = %error,
                "[conversations] blocking store task failed to join"
            );
            Err(format!(
                "conversation store {operation} task failed: {error}"
            ))
        }
    }
}

/// [`store::ensure_thread`] on the blocking pool.
pub async fn ensure_thread(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    request: CreateConversationThread,
) -> Result<ConversationThread, String> {
    run(ctx.clone(), "ensure_thread", move || {
        store::ensure_thread(&ctx, workspace_dir, request)
    })
    .await
}

/// [`store::list_threads`] on the blocking pool.
pub async fn list_threads(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
) -> Result<Vec<ConversationThread>, String> {
    run(ctx.clone(), "list_threads", move || {
        store::list_threads(&ctx, workspace_dir)
    })
    .await
}

/// [`super::get_messages`] on the blocking pool: the thread's messages without
/// artifact rows (see [`crate::threads::artifact_rows`]).
pub async fn get_messages(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
) -> Result<Vec<ConversationMessage>, String> {
    run(ctx.clone(), "get_messages", move || {
        store::get_messages(&ctx, workspace_dir, &thread_id)
    })
    .await
}

/// [`super::backend::get_messages`] on the blocking pool, artifact rows included. Only
/// for the UI's message list and the artifact-row writers.
pub async fn get_messages_with_artifact_rows(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
) -> Result<Vec<ConversationMessage>, String> {
    let thread_id = thread_id.to_string();
    // The backend's read keeps every row; `store::get_messages` drops the
    // artifact rows.
    run(ctx.clone(), "get_messages_with_artifact_rows", move || {
        super::backend::get_messages(&ctx, workspace_dir, &thread_id)
    })
    .await
}

/// [`store::append_message`] on the blocking pool.
pub async fn append_message(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    message: ConversationMessage,
) -> Result<ConversationMessage, String> {
    run(ctx.clone(), "append_message", move || {
        store::append_message(&ctx, workspace_dir, &thread_id, message)
    })
    .await
}

/// [`store::update_message`] on the blocking pool.
pub async fn update_message(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    message_id: String,
    patch: ConversationMessagePatch,
) -> Result<ConversationMessage, String> {
    run(ctx.clone(), "update_message", move || {
        store::update_message(&ctx, workspace_dir, &thread_id, &message_id, patch)
    })
    .await
}

/// [`store::delete_messages_from`] on the blocking pool.
pub async fn delete_messages_from(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    message_id: String,
) -> Result<Option<usize>, String> {
    run(ctx.clone(), "delete_messages_from", move || {
        store::delete_messages_from(&ctx, workspace_dir, &thread_id, &message_id)
    })
    .await
}

/// [`store::update_thread_title`] on the blocking pool.
pub async fn update_thread_title(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    title: String,
    updated_at: String,
) -> Result<ConversationThread, String> {
    run(ctx.clone(), "update_thread_title", move || {
        store::update_thread_title(&ctx, workspace_dir, &thread_id, &title, &updated_at)
    })
    .await
}

/// [`store::update_thread_working_dir`] on the blocking pool.
pub async fn update_thread_working_dir(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    working_dir: Option<String>,
    updated_at: String,
) -> Result<ConversationThread, String> {
    run(ctx.clone(), "update_thread_working_dir", move || {
        store::update_thread_working_dir(&ctx, workspace_dir, &thread_id, working_dir, &updated_at)
    })
    .await
}

/// [`store::update_thread_labels`] on the blocking pool.
pub async fn update_thread_labels(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    labels: Vec<String>,
    updated_at: String,
) -> Result<ConversationThread, String> {
    run(ctx.clone(), "update_thread_labels", move || {
        store::update_thread_labels(&ctx, workspace_dir, &thread_id, labels, &updated_at)
    })
    .await
}

/// [`store::delete_thread`] on the blocking pool.
pub async fn delete_thread(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    deleted_at: String,
) -> Result<bool, String> {
    run(ctx.clone(), "delete_thread", move || {
        store::delete_thread(&ctx, workspace_dir, &thread_id, &deleted_at)
    })
    .await
}

/// [`ConversationStore::transcript_key`] on the blocking pool: the session
/// store key `thread_id`'s transcripts were first written under, if recorded.
pub async fn transcript_key(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
) -> Result<Option<String>, String> {
    run(ctx.clone(), "transcript_key", move || {
        ConversationStore::new(ctx, workspace_dir).transcript_key(&thread_id)
    })
    .await
}

/// [`ConversationStore::record_transcript_key`] on the blocking pool: records
/// `key` unless the thread already has one, and returns the key in effect.
pub async fn record_transcript_key(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    key: String,
) -> Result<String, String> {
    run(ctx.clone(), "record_transcript_key", move || {
        ConversationStore::new(ctx, workspace_dir).record_transcript_key(&thread_id, &key)
    })
    .await
}

/// [`store::purge_threads`] on the blocking pool.
pub async fn purge_threads(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
) -> Result<ConversationPurgeStats, String> {
    run(ctx.clone(), "purge_threads", move || {
        store::purge_threads(&ctx, workspace_dir)
    })
    .await
}

/// [`ConversationStore::search_cross_thread_messages`] on the blocking pool.
///
/// The heaviest operation in the family: a cold inverted index reads every
/// thread's transcript before the search runs.
///
/// Artifact rows never come back: they only place cards in the UI, and these
/// hits feed search results and the agent's cross-chat context. The search asks
/// for extra hits so dropping rows still fills `limit`.
pub async fn search_cross_thread_messages(
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    query: String,
    limit: usize,
    exclude_thread_id: Option<String>,
) -> Result<Vec<CrossThreadHit>, String> {
    let wanted = limit.saturating_mul(2).max(limit.saturating_add(8));
    let hits = run(ctx.clone(), "search_cross_thread_messages", move || {
        ConversationStore::new(ctx, workspace_dir).search_cross_thread_messages(
            &query,
            wanted,
            exclude_thread_id.as_deref(),
        )
    })
    .await?;
    Ok(hits
        .into_iter()
        .filter(|hit| !crate::threads::artifact_rows::is_artifact_row_id(&hit.message_id))
        .take(limit)
        .collect())
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
