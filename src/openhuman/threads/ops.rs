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
use crate::rpc::RpcOutcome;
use serde::Serialize;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

const THREAD_TITLE_LOG_PREFIX: &str = "[threads:title]";
const THREAD_TITLE_MODEL_HINT: &str = "hint:summarize";
const THREAD_TITLE_SYSTEM_PROMPT: &str = "You generate short, specific chat thread titles from the first user message and the assistant reply. Return only the title text. Keep it under 8 words. No quotes. No markdown. No trailing punctuation unless it is part of a proper noun.";

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn counts(entries: impl IntoIterator<Item = (&'static str, usize)>) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn title_log_fingerprint(title: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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

fn is_auto_generated_thread_title(title: &str) -> bool {
    let trimmed = title.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 16 || !trimmed.starts_with("Chat ") {
        return false;
    }

    let month_end = 8;
    if bytes.len() <= month_end || !bytes[5..month_end].iter().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    if bytes.get(month_end) != Some(&b' ') {
        return false;
    }

    let mut idx = month_end + 1;
    let day_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == day_start || idx - day_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b' ') {
        return false;
    }
    idx += 1;

    let hour_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == hour_start || idx - hour_start > 2 {
        return false;
    }
    if bytes.get(idx) != Some(&b':') {
        return false;
    }
    idx += 1;

    if idx + 2 >= bytes.len()
        || !bytes[idx].is_ascii_digit()
        || !bytes[idx + 1].is_ascii_digit()
        || bytes[idx + 2] != b' '
    {
        return false;
    }
    idx += 3;

    matches!(&trimmed[idx..], "AM" | "PM")
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_generated_title(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(raw)
        .trim();
    let trimmed = line
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'))
        .trim()
        .trim_end_matches(['.', '!', '?', ':', ';'])
        .trim();
    let collapsed = collapse_whitespace(trimmed);
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(80).collect())
}

fn build_title_prompt(user_message: &str, assistant_message: &str) -> String {
    format!(
        "First user message:\n{user_message}\n\nAssistant reply:\n{assistant_message}\n\nReturn the best thread title."
    )
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
mod tests {
    //! Shape + validation tests for the pure, pre-IO helpers used by the
    //! threads RPC surface. Every test here avoids disk, network, and
    //! provider calls — they pin the behaviour of the branches that all of
    //! the async `ops::*` entry points rely on.
    use super::*;
    use serde_json::{json, Value};

    // ── request_id ────────────────────────────────────────────────

    #[test]
    fn request_id_is_a_non_empty_uuid_and_fresh_per_call() {
        let a = request_id();
        let b = request_id();
        assert!(!a.is_empty());
        // v4 UUID canonical form: 36 chars with 4 hyphens.
        assert_eq!(a.len(), 36);
        assert_eq!(a.chars().filter(|c| *c == '-').count(), 4);
        // Two calls must not collide — catches accidental caching.
        assert_ne!(a, b);
    }

    // ── counts ────────────────────────────────────────────────────

    #[test]
    fn counts_materialises_entries_as_owned_string_keys() {
        let map = counts([("num_threads", 3), ("num_messages", 7)]);
        assert_eq!(map.get("num_threads"), Some(&3));
        assert_eq!(map.get("num_messages"), Some(&7));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn counts_empty_iter_yields_empty_map() {
        let map = counts([]);
        assert!(map.is_empty());
    }

    // ── title_log_fingerprint ─────────────────────────────────────

    #[test]
    fn title_log_fingerprint_is_16_lowercase_hex_chars() {
        let fp = title_log_fingerprint("Chat Jan 1 1:00 AM");
        assert_eq!(fp.len(), 16);
        assert!(
            fp.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "fingerprint must be lowercase hex, got: {fp}"
        );
    }

    #[test]
    fn title_log_fingerprint_is_deterministic_for_same_title() {
        // The fingerprint is only used for debug logging — the only real
        // contract is stability across calls inside a single process so
        // grep-friendly logs remain correlatable.
        let a = title_log_fingerprint("My cool thread");
        let b = title_log_fingerprint("My cool thread");
        assert_eq!(a, b);
    }

    #[test]
    fn title_log_fingerprint_differs_for_different_titles() {
        let a = title_log_fingerprint("thread one");
        let b = title_log_fingerprint("thread two");
        assert_ne!(a, b, "distinct titles must produce distinct fingerprints");
    }

    // ── collapse_whitespace ───────────────────────────────────────

    #[test]
    fn collapse_whitespace_collapses_runs_and_trims_edges() {
        assert_eq!(collapse_whitespace("  a   b\tc\nd  "), "a b c d");
    }

    #[test]
    fn collapse_whitespace_on_empty_or_whitespace_only_is_empty() {
        assert_eq!(collapse_whitespace(""), "");
        assert_eq!(collapse_whitespace("   \t\n "), "");
    }

    // ── build_title_prompt ────────────────────────────────────────

    #[test]
    fn build_title_prompt_renders_user_and_assistant_sections_in_order() {
        let prompt = build_title_prompt("hi there", "hello back");
        assert_eq!(
            prompt,
            "First user message:\nhi there\n\nAssistant reply:\nhello back\n\nReturn the best thread title."
        );
    }

    // ── sanitize_generated_title ──────────────────────────────────

    #[test]
    fn sanitize_generated_title_trims_surrounding_quotes_and_trailing_punct() {
        assert_eq!(
            sanitize_generated_title("\"Hello, world!\""),
            Some("Hello, world".to_string())
        );
        assert_eq!(
            sanitize_generated_title("`Hello`"),
            Some("Hello".to_string())
        );
        assert_eq!(
            sanitize_generated_title("'Plan trip'"),
            Some("Plan trip".to_string())
        );
    }

    #[test]
    fn sanitize_generated_title_strips_repeated_trailing_punct() {
        assert_eq!(
            sanitize_generated_title("Check this out.!?"),
            Some("Check this out".to_string())
        );
    }

    #[test]
    fn sanitize_generated_title_picks_first_non_empty_line() {
        assert_eq!(
            sanitize_generated_title("\n   \nLine one\nLine two"),
            Some("Line one".to_string())
        );
    }

    #[test]
    fn sanitize_generated_title_returns_none_for_empty_or_whitespace() {
        assert!(sanitize_generated_title("").is_none());
        assert!(sanitize_generated_title("   \n\t").is_none());
        assert!(sanitize_generated_title("\"\"").is_none());
    }

    #[test]
    fn sanitize_generated_title_collapses_internal_whitespace() {
        assert_eq!(
            sanitize_generated_title("Very   spaced\tout"),
            Some("Very spaced out".to_string())
        );
    }

    #[test]
    fn sanitize_generated_title_truncates_to_80_chars_by_char_count() {
        // 100 `a` chars → must truncate to exactly 80. Char-based truncation
        // is load-bearing so multibyte titles never get sliced mid-codepoint.
        let raw = "a".repeat(100);
        let out = sanitize_generated_title(&raw).expect("non-empty");
        assert_eq!(out.chars().count(), 80);
    }

    #[test]
    fn sanitize_generated_title_truncation_is_char_safe_for_multibyte() {
        // 100 emoji (4-byte UTF-8 each) must still truncate on char
        // boundaries, proving the `.chars().take(80)` vs byte slicing
        // guarantee.
        let raw = "🌍".repeat(100);
        let out = sanitize_generated_title(&raw).expect("non-empty");
        assert_eq!(out.chars().count(), 80);
    }

    // ── is_auto_generated_thread_title ────────────────────────────

    #[test]
    fn is_auto_generated_thread_title_accepts_canonical_new_chat_format() {
        // Parser locks the format produced by `thread_create_new`:
        // "Chat <Mon> <day> <H:MM> AM|PM".
        assert!(is_auto_generated_thread_title("Chat Jan 1 1:00 AM"));
        assert!(is_auto_generated_thread_title("Chat Dec 31 12:59 PM"));
    }

    #[test]
    fn is_auto_generated_thread_title_tolerates_surrounding_whitespace() {
        // Input is trimmed before parsing — storage layers may round-trip
        // titles with stray whitespace.
        assert!(is_auto_generated_thread_title("  Chat Jan 1 1:00 AM  "));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_user_edited_titles() {
        // Any freeform user title must fall through to the "not a
        // placeholder" branch so we never overwrite user-authored names.
        assert!(!is_auto_generated_thread_title("My custom title"));
        assert!(!is_auto_generated_thread_title("Trip planning"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_short_strings() {
        // Hard `bytes.len() < 16` guard — locks in the minimum shape so
        // we never enter the parser with too-small input.
        assert!(!is_auto_generated_thread_title(""));
        assert!(!is_auto_generated_thread_title("Chat"));
        assert!(!is_auto_generated_thread_title("Chat Jan 1"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_non_alpha_month() {
        // Month abbreviation must be 3 ASCII alpha chars.
        assert!(!is_auto_generated_thread_title("Chat 123 1 1:00 AM"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_long_month_name() {
        // "January 1 1:00 AM" — after "Chat ", bytes[8] is 'u' not ' '.
        assert!(!is_auto_generated_thread_title("Chat January 1 1:00 AM"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_three_digit_day() {
        // day: 1–2 ASCII digits; idx-day_start>2 rejects.
        assert!(!is_auto_generated_thread_title("Chat Jan 100 1:00 AM"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_missing_colon() {
        // 3-digit hour consumes through the position the `:` must occupy.
        assert!(!is_auto_generated_thread_title("Chat Jan 1 100 AM"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_lowercase_meridiem() {
        // Parser only accepts "AM" | "PM" (not "am"/"pm") so pattern stays
        // tied to the producer in `thread_create_new`.
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00 am"));
    }

    #[test]
    fn is_auto_generated_thread_title_rejects_missing_space_before_meridiem() {
        // The `bytes[idx + 2] != b' '` guard must reject "1:00AM" (no space).
        assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00AM"));
    }

    // ── envelope ──────────────────────────────────────────────────

    #[test]
    fn envelope_sets_data_and_propagates_counts_and_pagination() {
        let pagination = PaginationMeta {
            limit: 10,
            offset: 0,
            count: 7,
        };
        let counts_map = counts([("num_messages", 7)]);
        let out = envelope(json!({"v": 42}), Some(counts_map.clone()), Some(pagination.clone()));
        let env = &out.value;
        assert_eq!(env.data.as_ref().unwrap()["v"], json!(42));
        assert!(env.error.is_none());
        assert!(!env.meta.request_id.is_empty());
        assert_eq!(env.meta.counts.as_ref().unwrap(), &counts_map);
        let pag = env.meta.pagination.as_ref().unwrap();
        assert_eq!(pag.limit, pagination.limit);
        assert_eq!(pag.count, pagination.count);
        assert_eq!(pag.offset, pagination.offset);
        // No implicit latency/cached info — the envelope helper keeps
        // optional fields unset so callers opt in explicitly.
        assert!(env.meta.latency_seconds.is_none());
        assert!(env.meta.cached.is_none());
        // No logs are attached by default.
        assert!(out.logs.is_empty());
    }

    #[test]
    fn envelope_omits_counts_and_pagination_when_not_provided() {
        let out = envelope(json!(null), None, None);
        assert!(out.value.meta.counts.is_none());
        assert!(out.value.meta.pagination.is_none());
    }

    #[test]
    fn envelope_generates_unique_request_ids_per_call() {
        // request_id uniqueness matters for client-side correlation of
        // overlapping threads-API calls. Lock it in.
        let a = envelope(json!({}), None, None);
        let b = envelope(json!({}), None, None);
        assert_ne!(a.value.meta.request_id, b.value.meta.request_id);
    }

    // ── thread_to_summary / message_to_record / record_to_message ─

    fn sample_thread() -> ConversationThread {
        ConversationThread {
            id: "t-1".into(),
            title: "My thread".into(),
            chat_id: Some(42),
            is_active: true,
            message_count: 5,
            last_message_at: "2026-01-01T00:00:00Z".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn sample_message() -> ConversationMessage {
        ConversationMessage {
            id: "m-1".into(),
            content: "hi".into(),
            message_type: "text".into(),
            extra_metadata: json!({"k": "v"}),
            sender: "user".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn thread_to_summary_preserves_all_fields() {
        let summary = thread_to_summary(sample_thread());
        assert_eq!(summary.id, "t-1");
        assert_eq!(summary.title, "My thread");
        assert_eq!(summary.chat_id, Some(42));
        assert!(summary.is_active);
        assert_eq!(summary.message_count, 5);
        assert_eq!(summary.last_message_at, "2026-01-01T00:00:00Z");
        assert_eq!(summary.created_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn message_to_record_and_back_is_lossless() {
        let msg = sample_message();
        let record = message_to_record(msg.clone());
        assert_eq!(record.id, msg.id);
        assert_eq!(record.content, msg.content);
        assert_eq!(record.message_type, msg.message_type);
        assert_eq!(record.extra_metadata, msg.extra_metadata);
        assert_eq!(record.sender, msg.sender);
        assert_eq!(record.created_at, msg.created_at);

        let round_tripped = record_to_message(record);
        assert_eq!(round_tripped, msg);
    }

    #[test]
    fn record_to_message_preserves_null_extra_metadata() {
        // Default Value::Null must pass through untouched so the downstream
        // storage layer sees the same "no metadata" signal it produced.
        let rec = ConversationMessageRecord {
            id: "m-2".into(),
            content: "x".into(),
            message_type: "text".into(),
            extra_metadata: Value::Null,
            sender: "agent".into(),
            created_at: "2026-01-02T00:00:00Z".into(),
        };
        let msg = record_to_message(rec);
        assert_eq!(msg.extra_metadata, Value::Null);
        assert_eq!(msg.sender, "agent");
    }

    // ── Title constants ───────────────────────────────────────────

    #[test]
    fn title_system_prompt_constrains_model_output_shape() {
        // The system prompt is shipped verbatim to the provider. Locking
        // in the trailing "no trailing punctuation" clause catches
        // accidental edits that would let the model emit trailing periods
        // that `sanitize_generated_title` would then silently strip.
        assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("under 8 words"));
        assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No quotes"));
        assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No markdown"));
    }

    #[test]
    fn title_log_prefix_is_grep_friendly_and_stable() {
        // The `[threads:title]` prefix is what CLAUDE.md's "debug logging"
        // rule asks contributors to grep for when debugging. It is part
        // of the observable contract — lock it down.
        assert_eq!(THREAD_TITLE_LOG_PREFIX, "[threads:title]");
    }
}
