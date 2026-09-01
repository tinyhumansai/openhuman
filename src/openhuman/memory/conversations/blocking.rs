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
//! now [`super`]'s own subtree rather than `tinycortex::memory::conversations`,
//! and the import below is the only line that changed for it. The item set is
//! the same one the engine exported, so function signatures, argument order,
//! error strings and the `[conversations]` log prefix stay byte-identical —
//! `web_chat::run_task` and the RPC layer read them.
//!
//! Every store entry point is synchronous, and each one takes the
//! process-global `CONVERSATION_STORE_LOCK` — a
//! `parking_lot::Mutex` — and then does fsync'd JSONL file IO while holding it.
//! Calling one directly from an `async fn` therefore parks a tokio **worker**
//! thread for the whole wait, and the wait is not short:
//!
//! * `threads.jsonl` is folded from scratch on nearly every operation
//!   (`thread_index_unlocked`), and it grows by roughly two lines per appended
//!   message and is never compacted — so the per-call fold cost grows with the
//!   user's whole history;
//! * `append_message` writes two fsync'd appends under the lock, and
//!   `update_message` reads and rewrites a thread's entire message log under it,
//!   so a live streaming turn holds the lock repeatedly;
//! * `search_cross_thread_messages` reads every thread's transcript on a cold
//!   index.
//!
//! Once more concurrent conversation operations than there are worker threads
//! are parked on that mutex, the runtime stops polling **anything** — including
//! the HTTP task that owes the client its response. That is how a create that
//! only needs one append blows the frontend's 30 s RPC budget:
//! `UnhandledRejection: Core RPC openhuman.threads_create_new timed out after
//! 30000ms` (Sentry TAURI-REACT-10, #5156).
//!
//! Moving each call onto the blocking pool keeps the lock wait off the async
//! workers. The work is just as serialized as before — the store's lock still
//! decides who writes when — but the executor stays live, so the RPC server
//! keeps answering while a slow conversation operation drains, and a queued
//! create completes as soon as the lock frees instead of after the client has
//! given up.
//!
//! Callers pass owned arguments because the closure must be `'static`.

use std::path::PathBuf;

use crate::openhuman::memory::conversations as store;

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
async fn run<T, F>(operation: &'static str, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
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
    workspace_dir: PathBuf,
    request: CreateConversationThread,
) -> Result<ConversationThread, String> {
    run("ensure_thread", move || {
        store::ensure_thread(workspace_dir, request)
    })
    .await
}

/// [`store::list_threads`] on the blocking pool.
pub async fn list_threads(workspace_dir: PathBuf) -> Result<Vec<ConversationThread>, String> {
    run("list_threads", move || store::list_threads(workspace_dir)).await
}

/// [`store::get_messages`] on the blocking pool.
pub async fn get_messages(
    workspace_dir: PathBuf,
    thread_id: String,
) -> Result<Vec<ConversationMessage>, String> {
    run("get_messages", move || {
        store::get_messages(workspace_dir, &thread_id)
    })
    .await
}

/// [`store::append_message`] on the blocking pool.
pub async fn append_message(
    workspace_dir: PathBuf,
    thread_id: String,
    message: ConversationMessage,
) -> Result<ConversationMessage, String> {
    run("append_message", move || {
        store::append_message(workspace_dir, &thread_id, message)
    })
    .await
}

/// [`store::update_message`] on the blocking pool.
pub async fn update_message(
    workspace_dir: PathBuf,
    thread_id: String,
    message_id: String,
    patch: ConversationMessagePatch,
) -> Result<ConversationMessage, String> {
    run("update_message", move || {
        store::update_message(workspace_dir, &thread_id, &message_id, patch)
    })
    .await
}

/// [`store::update_thread_title`] on the blocking pool.
pub async fn update_thread_title(
    workspace_dir: PathBuf,
    thread_id: String,
    title: String,
    updated_at: String,
) -> Result<ConversationThread, String> {
    run("update_thread_title", move || {
        store::update_thread_title(workspace_dir, &thread_id, &title, &updated_at)
    })
    .await
}

/// [`store::update_thread_labels`] on the blocking pool.
pub async fn update_thread_labels(
    workspace_dir: PathBuf,
    thread_id: String,
    labels: Vec<String>,
    updated_at: String,
) -> Result<ConversationThread, String> {
    run("update_thread_labels", move || {
        store::update_thread_labels(workspace_dir, &thread_id, labels, &updated_at)
    })
    .await
}

/// [`store::delete_thread`] on the blocking pool.
pub async fn delete_thread(
    workspace_dir: PathBuf,
    thread_id: String,
    deleted_at: String,
) -> Result<bool, String> {
    run("delete_thread", move || {
        store::delete_thread(workspace_dir, &thread_id, &deleted_at)
    })
    .await
}

/// [`store::purge_threads`] on the blocking pool.
pub async fn purge_threads(workspace_dir: PathBuf) -> Result<ConversationPurgeStats, String> {
    run("purge_threads", move || store::purge_threads(workspace_dir)).await
}

/// [`ConversationStore::search_cross_thread_messages`] on the blocking pool.
///
/// The heaviest operation in the family: a cold inverted index reads every
/// thread's transcript before the search runs.
pub async fn search_cross_thread_messages(
    workspace_dir: PathBuf,
    query: String,
    limit: usize,
    exclude_thread_id: Option<String>,
) -> Result<Vec<CrossThreadHit>, String> {
    run("search_cross_thread_messages", move || {
        ConversationStore::new(workspace_dir).search_cross_thread_messages(
            &query,
            limit,
            exclude_thread_id.as_deref(),
        )
    })
    .await
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
