//! Shared helpers used across the threads RPC operations: response envelope
//! construction, workspace resolution, cancellation-safe execution, and the
//! store-type <-> wire-type conversions every handler needs.

use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DispatchCtx;
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

/// Where a thread's session transcripts live for one request: the workspace
/// and the agent key the session host stored them under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptStore {
    pub workspace_dir: PathBuf,
    /// The store key ([`agent_key_for`](crate::agent::session_store::agent_key_for)):
    /// `<profile>~<agent>` for a SaaS profile, else the agent, else the chat agent id.
    pub key: String,
}

impl TranscriptStore {
    /// The transcript port for this key: the installed session store, else
    /// the workspace's own store.
    pub(crate) fn locator(
        &self,
    ) -> std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
        crate::agent::session_store::transcripts_or_files(&self.key, &self.workspace_dir)
    }
}

/// The transcript store of `thread_id` for the request `ctx` carries: the key
/// the thread's transcripts were first written under, recorded with the thread
/// ([`ConversationStore::record_transcript_key`](crate::threads::store::ConversationStore::record_transcript_key)),
/// so a later `chat_agent_id` change cannot strand its earlier turns. A thread
/// with no record resolves as the session host would today: the config's chat
/// agent as the fallback id, and the request's own tenant (never the ambient
/// one), so a SaaS profile's key keeps its profile prefix.
///
/// # Errors
///
/// The config cannot be loaded, the request has no tenant in SaaS mode, or the
/// recorded key cannot be read.
pub(crate) async fn transcript_store(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
) -> Result<TranscriptStore, String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let tenant = crate::core::runtime::tenant::tenant_in(
        crate::core::runtime::is_saas(),
        ctx.resolved().as_deref(),
    )
    .map_err(|no_tenant| no_tenant.to_string())?;
    let mut store = transcript_store_for(&config, &tenant);
    let recorded = crate::threads::store::blocking::transcript_key(
        ctx.clone(),
        config.workspace_dir.clone(),
        thread_id.to_string(),
    )
    .await?;
    if let Some(key) = recorded {
        log::debug!(
            "[threads] transcript store key from the thread's recorded key (differs_from_current_rule={})",
            key != store.key
        );
        store.key = key;
    }
    Ok(store)
}

/// [`transcript_store`] for an already-resolved config and tenant.
pub(crate) fn transcript_store_for(
    config: &Config,
    tenant: &crate::core::runtime::Tenant,
) -> TranscriptStore {
    let key = crate::agent::session_store::agent_key_for(
        tenant,
        &crate::web_chat::pick_target_agent_id(config),
    );
    log::debug!(
        "[threads] transcript store key resolved profile={} agent={}",
        tenant.profile.is_some(),
        tenant.agent.is_some()
    );
    TranscriptStore {
        workspace_dir: config.workspace_dir.clone(),
        key,
    }
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
/// sequence from the caller's lifetime. The caller's [`CoreContext`] is moved
/// into the task explicitly: a bare `tokio::spawn` drops the `task_local` scope,
/// and the task would silently fall back to the process default — which under
/// multi-tenant scoped dispatch is the wrong workspace.
pub(super) async fn run_to_completion<T, F>(
    dispatch: &crate::core::runtime::DispatchCtx,
    operation: &'static str,
    fut: F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let ctx = dispatch.core().cloned();
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
        action_dir: thread.working_dir,
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
    ctx: &DispatchCtx,
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
        ctx.clone(),
        dir,
        thread.id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
}
