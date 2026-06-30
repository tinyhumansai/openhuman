//! RPC operations for conversation thread management.

use crate::openhuman::channels::providers::web as web_channel;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::{self, ProviderRuntimeOptions};
use crate::openhuman::memory::{
    ApiEnvelope, ApiMeta, AppendConversationMessageRequest, ConversationMessageRecord,
    ConversationMessagesRequest, ConversationMessagesResponse, ConversationThreadSummary,
    ConversationThreadsListResponse, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, DeleteConversationThreadResponse, EmptyRequest,
    GenerateConversationThreadTitleRequest, PaginationMeta, PurgeConversationThreadsResponse,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};
use crate::openhuman::memory_conversations::{
    self as conversations, ConversationMessage, ConversationMessagePatch, ConversationStore,
    ConversationThread, CreateConversationThread, CrossThreadHit,
};
use crate::openhuman::threads::title::{
    build_title_prompt, is_auto_generated_thread_title, sanitize_generated_title,
    title_from_user_message, title_log_fingerprint, THREAD_TITLE_LOG_PREFIX,
    THREAD_TITLE_MODEL_HINT, THREAD_TITLE_SYSTEM_PROMPT,
};
use crate::openhuman::threads::turn_state::{
    self, ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateRequest, GetTurnStateResponse,
    ListTurnStatesResponse,
};
use crate::openhuman::threads::ThreadsError;
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

fn update_thread_with_fallback_title(
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
    conversations::update_thread_title(dir, &thread.id, &title, &chrono::Utc::now().to_rfc3339())
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
            parent_thread_id: request.parent_thread_id,
            labels: request.labels,
            personality_id: request.personality_id,
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
    request: CreateConversationThreadRequest,
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
            parent_thread_id: None,
            // Pass labels through as-is; the store's infer_labels() applies
            // the same default on index rebuild, so this is the single source
            // of truth for default labels.
            labels: request.labels,
            personality_id: request.personality_id,
        },
    )?;
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
    let hits = ConversationStore::new(dir).search_cross_thread_messages(
        query,
        limit,
        exclude_thread_id,
    )?;
    log::debug!("[threads][transcript_search] hits={}", hits.len());
    Ok(hits)
}

/// Appends a message to a conversation thread.
pub async fn message_append(
    request: AppendConversationMessageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
    let dir = workspace_dir().await?;
    let message =
        conversations::append_message(dir, &request.thread_id, record_to_message(request.message))
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
    let Some(thread) = conversations::list_threads(dir.clone())?
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
            "{THREAD_TITLE_LOG_PREFIX} no assistant message yet; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message)?;
        return Ok(envelope(
            thread_to_summary(updated),
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

    let provider = match provider::create_intelligent_routing_provider(
        config.inference_url.as_deref(),
        config.api_url.as_deref(),
        config.api_key.as_deref(),
        &config,
        &provider_runtime_options,
    ) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} provider init failed; applying fallback title"
            );
            let updated = update_thread_with_fallback_title(dir, thread, &first_user_message)?;
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
                "{THREAD_TITLE_LOG_PREFIX} title generation failed; applying fallback title"
            );
            let updated = update_thread_with_fallback_title(dir, thread, &first_user_message)?;
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
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message)?;
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

    let updated = conversations::update_thread_title(
        dir,
        &request.thread_id,
        &title,
        &chrono::Utc::now().to_rfc3339(),
    )
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
    let thread = conversations::update_thread_labels(
        dir,
        &request.thread_id,
        request.labels.clone(),
        &chrono::Utc::now().to_rfc3339(),
    )?;
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
    let updated = conversations::update_thread_title(
        dir,
        &request.thread_id,
        &title,
        &chrono::Utc::now().to_rfc3339(),
    )
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
    let deleted = ConversationStore::new(dir.clone())
        .delete_thread(&request.thread_id, &request.deleted_at)?;
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
    let cancelled = crate::openhuman::agent_orchestration::running_subagents::cancel_for_thread(
        &request.thread_id,
    );
    let discarded =
        crate::openhuman::agent_orchestration::background_completions::discard_for_thread(
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
pub async fn threads_purge(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    let stats = conversations::purge_threads(dir.clone())?;
    // No parent thread survives a purge, so cancel every detached sub-agent and
    // wipe every queued result. Same ordering as `thread_delete`: abort the
    // in-flight runs first, then clear the delivery queue. Tombstone each
    // cancelled sub-agent's thread BEFORE the final wipe so a straggler that
    // wins the cooperative-abort race (records after the wipe) is still dropped
    // by `record_completion` rather than delivered into a purged thread.
    use crate::openhuman::agent_orchestration::{background_completions, running_subagents};
    let cancelled_threads = running_subagents::cancel_all();
    let mut discarded = 0;
    for thread_id in &cancelled_threads {
        discarded += background_completions::discard_for_thread(thread_id);
    }
    discarded += background_completions::clear_all();
    log::debug!(
        "[threads] threads_purge cancelled_threads={} discarded_completions={}",
        cancelled_threads.len(),
        discarded
    );
    // Threads are gone, so any orphan turn snapshots can never be
    // reattached to a live thread. Wipe them in the same call so
    // `turn_state_list` returns an empty set after a purge. Use the
    // parse-independent `clear_all` so corrupted / half-written
    // snapshot files (which `list()` would warn-and-skip) are also
    // removed — a destructive cleanup must not leave behind anything
    // it failed to deserialize. Failures surface as RPC errors.
    turn_state::store::clear_all(dir.clone())
        .map_err(|err| format!("threads purged but turn-snapshot cleanup failed: {err}"))?;
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

/// Returns the persisted in-flight turn snapshot for a thread, if any.
pub async fn turn_state_get(
    request: GetTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get(dir, &request.thread_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Lists every persisted turn snapshot — used by the UI on cold boot to
/// surface interrupted turns from a previous process.
pub async fn turn_state_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list(dir)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Clears the persisted turn snapshot for a thread (e.g. after the user
/// dismisses an "interrupted" banner).
pub async fn turn_state_clear(
    request: ClearTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<ClearTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let cleared = turn_state::store::delete(dir, &request.thread_id)?;
    Ok(envelope(ClearTurnStateResponse { cleared }, None, None))
}

/// Request for [`token_usage`]: the thread whose persisted usage to total.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadTokenUsageRequest {
    pub thread_id: String,
}

/// Aggregated token/cost usage for one thread, read back from its persisted
/// session transcripts. Seeds the UI footer when the user selects a thread so
/// the totals reflect prior turns instead of starting at zero.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTokenUsageResponse {
    pub thread_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub turn_count: usize,
    /// Tokens of the most recent turn — numerator for the context-window gauge.
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Context window (tokens) inferred from the last model; `0` when unknown.
    pub context_window: u64,
    pub model: Option<String>,
    pub updated: Option<String>,
    /// `false` when the thread has no persisted turns yet (all zeros).
    pub has_usage: bool,
    /// Per-archetype sub-agent spend (re-audited at current pricing). The
    /// top-level totals already include this; it's broken out for the UI's
    /// per-agent footer rows.
    pub subagents: Vec<SubagentUsageDto>,
}

/// One sub-agent archetype's contribution within a thread.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentUsageDto {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub runs: usize,
}

/// Total a thread's persisted token/cost usage across its root transcripts.
pub async fn token_usage(
    request: ThreadTokenUsageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ThreadTokenUsageResponse>>, String> {
    let dir = workspace_dir().await?;
    let summary = crate::openhuman::agent::harness::session::transcript::read_thread_usage_summary(
        &dir,
        &request.thread_id,
    );

    // Re-audit cost at CURRENT pricing rather than trusting the
    // `charged_amount_usd` persisted in the transcript: those values were
    // stamped at turn time and don't reflect later tier-pricing corrections.
    // Recompute from the persisted token counts using the last-known model's
    // rates; falls back to `fallback` only when the model is unknown.
    let audit_cost =
        |model: Option<&str>, input: u64, output: u64, cached: u64, fallback: f64| match model {
            Some(m) => crate::openhuman::agent::cost::estimate_call_cost_usd(
                m,
                &crate::openhuman::inference::provider::UsageInfo {
                    input_tokens: input,
                    output_tokens: output,
                    cached_input_tokens: cached,
                    ..Default::default()
                },
            ),
            None => fallback,
        };

    let response = match summary {
        Some(s) => {
            let context_window = s
                .model
                .as_deref()
                .and_then(crate::openhuman::inference::model_context::context_window_for_model)
                .unwrap_or(0);

            // Orchestrator (root) spend, re-audited.
            let orchestrator_cost = audit_cost(
                s.model.as_deref(),
                s.input_tokens,
                s.output_tokens,
                s.cached_input_tokens,
                s.cost_usd,
            );

            // Sub-agent archetypes, each re-audited with its own model. Older
            // sub-agent transcripts didn't persist a model on their messages, so
            // fall back to the thread's (root) model rather than pricing them at
            // $0 — sub-agents usually run on the same managed tier as the parent.
            let mut subagents = Vec::with_capacity(s.subagents.len());
            let (mut sub_in, mut sub_out, mut sub_cached, mut sub_cost) = (0u64, 0u64, 0u64, 0.0);
            for g in &s.subagents {
                let sub_model = g.model.as_deref().or(s.model.as_deref());
                let cost = audit_cost(
                    sub_model,
                    g.input_tokens,
                    g.output_tokens,
                    g.cached_input_tokens,
                    0.0,
                );
                sub_in = sub_in.saturating_add(g.input_tokens);
                sub_out = sub_out.saturating_add(g.output_tokens);
                sub_cached = sub_cached.saturating_add(g.cached_input_tokens);
                sub_cost += cost;
                subagents.push(SubagentUsageDto {
                    agent_id: g.agent_id.clone(),
                    input_tokens: g.input_tokens,
                    output_tokens: g.output_tokens,
                    cost_usd: cost,
                    runs: g.runs,
                });
            }

            // Top-level totals = orchestrator + all sub-agents.
            ThreadTokenUsageResponse {
                thread_id: request.thread_id.clone(),
                input_tokens: s.input_tokens.saturating_add(sub_in),
                output_tokens: s.output_tokens.saturating_add(sub_out),
                cached_input_tokens: s.cached_input_tokens.saturating_add(sub_cached),
                cost_usd: orchestrator_cost + sub_cost,
                turn_count: s.turn_count,
                last_turn_input_tokens: s.last_turn_input_tokens,
                last_turn_output_tokens: s.last_turn_output_tokens,
                context_window,
                model: s.model,
                updated: Some(s.updated),
                has_usage: true,
                subagents,
            }
        }
        None => ThreadTokenUsageResponse {
            thread_id: request.thread_id.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            cost_usd: 0.0,
            turn_count: 0,
            last_turn_input_tokens: 0,
            last_turn_output_tokens: 0,
            context_window: 0,
            model: None,
            updated: None,
            has_usage: false,
            subagents: Vec::new(),
        },
    };

    let has_usage = response.has_usage;
    Ok(envelope(
        response,
        Some(counts([("has_usage", usize::from(has_usage))])),
        None,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
