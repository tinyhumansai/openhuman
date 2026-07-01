//! RPC handlers for the `agent_meetings` domain.
//!
//! Each handler emits a Socket.IO event to the backend via the global
//! `SocketManager`. The backend's meeting bot handler picks these up and
//! drives the Recall.ai (or Camoufox) session.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::meet::ops::validate_display_name;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::socket::global_socket_manager;
use crate::rpc::RpcOutcome;

use super::types::{
    BackendMeetHarnessResponseRequest, BackendMeetJoinRequest, BackendMeetJoinResponse,
    BackendMeetLeaveRequest, BackendMeetSpeakRequest, GenerateSummaryRequest,
    GenerateSummaryResponse, MeetingSessionStatus,
};

const ALLOWED_HOSTS: &[(&str, &str)] = &[
    ("meet.google.com", "gmeet"),
    ("zoom.us", "zoom"),
    ("teams.microsoft.com", "teams"),
    ("webex.com", "webex"),
];

// ---------------------------------------------------------------------------
// Phase 3 policy helpers
// ---------------------------------------------------------------------------

/// Map `AutoJoinPolicy` → the compact string used by the frontend and the
/// per-event policy store ("auto" | "ask" | "skip").
pub(crate) fn auto_join_policy_to_str(
    p: &crate::openhuman::config::schema::AutoJoinPolicy,
) -> &'static str {
    use crate::openhuman::config::schema::AutoJoinPolicy;
    match p {
        AutoJoinPolicy::Always => "auto",
        AutoJoinPolicy::AskEachTime => "ask",
        AutoJoinPolicy::Never => "skip",
    }
}

/// Map the compact policy string back to `AutoJoinPolicy`. Returns `None` for
/// unrecognised strings.
pub(crate) fn str_to_auto_join_policy(
    s: &str,
) -> Option<crate::openhuman::config::schema::AutoJoinPolicy> {
    use crate::openhuman::config::schema::AutoJoinPolicy;
    match s {
        "auto" => Some(AutoJoinPolicy::Always),
        "ask" => Some(AutoJoinPolicy::AskEachTime),
        "skip" => Some(AutoJoinPolicy::Never),
        _ => None,
    }
}

/// Resolve the effective join policy for a meeting, applying a three-tier
/// precedence:
///
/// 1. **Per-event override** — stored by `openhuman.meet_set_event_policy`.
/// 2. **Per-platform default** — from `config.meet.platform_auto_join_policies`.
/// 3. **Global default** — from `config.meet.auto_join_policy`.
///
/// Returns a compact string: "auto" | "ask" | "skip".
pub(crate) fn resolve_effective_join_policy(
    calendar_event_id: Option<&str>,
    platform: Option<&str>,
    config: &crate::openhuman::config::Config,
) -> String {
    // Single-event path: fetch this one override (opens one connection) and
    // delegate to the prefetch-aware resolver so the tier logic lives in exactly
    // one place.
    let mut overrides: HashMap<String, String> = HashMap::new();
    if let Some(event_id) = calendar_event_id {
        match super::store::get_event_policy(config, event_id) {
            Ok(Some(policy)) if !policy.is_empty() => {
                overrides.insert(event_id.to_string(), policy);
            }
            Err(e) => {
                tracing::debug!(
                    event_id,
                    error = %e,
                    "[meet:policy] per-event lookup failed, falling through"
                );
            }
            _ => {}
        }
    }
    resolve_effective_join_policy_with_overrides(calendar_event_id, platform, config, &overrides)
}

/// Prefetch-aware variant of [`resolve_effective_join_policy`].
///
/// Takes a map of per-event overrides already loaded from the store (see
/// [`super::store::get_event_policies_batch`]) so a batch RPC like
/// `handle_list_upcoming` can resolve every meeting's effective policy fully
/// in-memory — no per-event SQLite connection / schema migration. Applies the
/// same three-tier precedence: per-event override → per-platform → global.
pub(crate) fn resolve_effective_join_policy_with_overrides(
    calendar_event_id: Option<&str>,
    platform: Option<&str>,
    config: &crate::openhuman::config::Config,
    event_overrides: &HashMap<String, String>,
) -> String {
    // Tier 1: per-event override (from the prefetched map).
    if let Some(event_id) = calendar_event_id {
        if let Some(policy) = event_overrides.get(event_id) {
            if !policy.is_empty() {
                tracing::debug!(
                    event_id,
                    policy = %policy,
                    "[meet:policy] tier=per_event"
                );
                return policy.clone();
            }
        }
    }

    // Tier 2: per-platform default.
    if let Some(plat) = platform {
        if let Some(policy) = config.meet.platform_auto_join_policies.get(plat) {
            let s = auto_join_policy_to_str(policy);
            tracing::debug!(
                platform = plat,
                policy = s,
                "[meet:policy] tier=per_platform"
            );
            return s.to_string();
        }
    }

    // Tier 3: global default.
    let s = auto_join_policy_to_str(&config.meet.auto_join_policy);
    tracing::debug!(policy = s, "[meet:policy] tier=global");
    s.to_string()
}

fn transcript_turns_to_chat_batch(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
) -> Option<ChatBatch> {
    // Cap at 48 h to avoid DateTime underflow; real meetings never exceed this.
    const MAX_DURATION_MS: u64 = 172_800_000;
    let duration_i64 = i64::try_from(duration_ms.min(MAX_DURATION_MS)).unwrap_or(172_800_000);
    let base = chrono::Utc::now() - chrono::Duration::milliseconds(duration_i64);
    // Spread turns evenly across the duration; fall back to 1 ms spacing when
    // duration is zero or turns is empty (avoids division by zero).
    let spacing_ms = if turns.is_empty() {
        1i64
    } else {
        i64::try_from(duration_ms / turns.len() as u64).unwrap_or(1)
    };
    let mut messages = Vec::new();

    for (idx, turn) in turns.iter().enumerate() {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let author = if turn.role.eq_ignore_ascii_case("assistant") {
            "Tiny"
        } else {
            "Meeting participant"
        };
        let offset_ms = spacing_ms.saturating_mul(idx as i64);
        messages.push(ChatMessage {
            author: author.to_string(),
            timestamp: base + chrono::Duration::milliseconds(offset_ms),
            text: text.to_string(),
            source_ref: Some(format!("backend-meet://turn/{idx}")),
        });
    }

    if messages.is_empty() {
        None
    } else {
        Some(ChatBatch {
            platform: "backend_meet".to_string(),
            channel_label: "Recall AI meeting".to_string(),
            messages,
        })
    }
}

pub async fn ingest_backend_meeting_transcript(
    turns: Vec<BackendMeetTurn>,
    duration_ms: u64,
    correlation_id: Option<String>,
) -> Result<(), String> {
    let Some(batch) = transcript_turns_to_chat_batch(&turns, duration_ms) else {
        tracing::debug!("[agent_meetings] transcript had no ingestible turns");
        return Ok(());
    };

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("[agent_meetings] config load failed: {e}"))?;
    let cid_suffix = correlation_id.as_deref().unwrap_or("none");
    let source_id = format!(
        "meet:recall:{}:{}",
        chrono::Utc::now().timestamp_millis(),
        cid_suffix
    );
    let tags = vec!["meeting".to_string(), "recall_ai".to_string()];
    let result = ingest_pipeline::ingest_chat(&config, &source_id, "user", tags, batch)
        .await
        .map_err(|e| format!("[agent_meetings] transcript ingest failed: {e:#}"))?;

    tracing::info!(
        source_id = %source_id,
        chunks_written = result.chunks_written,
        correlation_id = ?correlation_id,
        "[agent_meetings] transcript ingested into memory tree"
    );

    Ok(())
}

/// Whether thread creation may perform its own summary generation when the
/// caller did not pass a pre-generated summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryGenerationMode {
    /// Preserve the legacy behavior: enrich transcript threads when possible.
    GenerateIfMissing,
    /// Append only a summary supplied by the caller; never invoke the LLM.
    UseProvidedOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadAppendMode {
    BestEffort,
    Strict,
}

/// Create a conversation thread labelled "Meetings" containing the transcript.
///
/// The correlation_id (when present) is embedded in the transcript body as an
/// external reference for tracing — it does not deduplicate; each call creates
/// a new thread.
/// `generated` lets the caller pass a summary it already produced so the
/// call-end pipeline can share one generation across the recent-call detail
/// store and this thread. When `None`, the summary is generated here (bounded)
/// — preserving the behaviour of callers that don't pre-generate.
pub async fn create_meeting_thread_with_transcript(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<String>,
    generated: Option<&super::summary::GeneratedSummary>,
) -> Result<String, String> {
    create_meeting_thread_with_transcript_with_summary_mode(
        turns,
        duration_ms,
        correlation_id,
        generated,
        SummaryGenerationMode::GenerateIfMissing,
    )
    .await
}

pub async fn create_meeting_thread_with_transcript_with_summary_mode(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<String>,
    generated: Option<&super::summary::GeneratedSummary>,
    summary_mode: SummaryGenerationMode,
) -> Result<String, String> {
    create_meeting_thread_with_transcript_inner(
        turns,
        duration_ms,
        correlation_id,
        generated,
        summary_mode,
        ThreadAppendMode::BestEffort,
    )
    .await
}

async fn create_meeting_thread_with_transcript_with_summary_mode_strict(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<String>,
    generated: Option<&super::summary::GeneratedSummary>,
    summary_mode: SummaryGenerationMode,
) -> Result<String, String> {
    create_meeting_thread_with_transcript_inner(
        turns,
        duration_ms,
        correlation_id,
        generated,
        summary_mode,
        ThreadAppendMode::Strict,
    )
    .await
}

async fn create_meeting_thread_with_transcript_inner(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<String>,
    generated: Option<&super::summary::GeneratedSummary>,
    summary_mode: SummaryGenerationMode,
    append_mode: ThreadAppendMode,
) -> Result<String, String> {
    use crate::openhuman::memory::{
        AppendConversationMessageRequest, ConversationMessageRecord,
        CreateConversationThreadRequest, UpdateConversationThreadTitleRequest,
    };
    use crate::openhuman::threads::ops;

    if turns.is_empty() {
        return Err(
            "[agent_meetings] cannot create a meeting thread without transcript turns".to_string(),
        );
    }

    // Format the transcript body first — this is the durable artifact and must
    // not depend on (or wait on) the summarisation LLM call.
    let mut body = String::new();
    let duration_min = duration_ms / 60_000;
    body.push_str(&format!("Duration: {duration_min} min\n\n"));
    if let Some(cid) = &correlation_id {
        body.push_str(&format!("Correlation ID: {cid}\n\n"));
    }
    for turn in turns {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let role_label = if turn.role.eq_ignore_ascii_case("assistant") {
            "Assistant"
        } else {
            "Participant"
        };
        body.push_str(&format!("**{role_label}**: {text}\n\n"));
    }

    // 1. Create the thread under the shared "Meetings" label and append the
    //    transcript *before* any LLM work, so thread/transcript persistence (and
    //    the memory-tree ingest that runs after this returns) never gate on
    //    summarisation. The per-meeting topic is applied later as the thread
    //    *title* only — adding it as a second label would accrue a unique,
    //    never-reused label per call and pollute the shared label taxonomy,
    //    while the title already disambiguates calls in the list.
    let create_req = CreateConversationThreadRequest {
        labels: Some(vec!["Meetings".to_string()]),
        personality_id: None,
    };
    let outcome = ops::thread_create_new(create_req)
        .await
        .map_err(|e| format!("[agent_meetings] thread creation failed: {e}"))?;
    let thread_id = outcome
        .value
        .data
        .as_ref()
        .ok_or_else(|| "[agent_meetings] thread creation returned no data".to_string())?
        .id
        .clone();

    // 2. Append the transcript as a message. The durable record is now complete
    //    regardless of whether summarisation succeeds below.
    let msg = ConversationMessageRecord {
        id: uuid::Uuid::new_v4().to_string(),
        content: body,
        message_type: "system".to_string(),
        extra_metadata: serde_json::Value::Null,
        sender: "system".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let append_req = AppendConversationMessageRequest {
        thread_id: thread_id.clone(),
        message: msg,
    };
    if let Err(e) = ops::message_append(append_req).await {
        let message = format!("[agent_meetings] failed to append transcript message: {e}");
        if matches!(append_mode, ThreadAppendMode::Strict) {
            return Err(message);
        }
        tracing::warn!(
            thread_id = %thread_id,
            "{message}"
        );
    }

    // 3. Best-effort enrichment: reuse a summary the caller already generated
    //    (the call-end pipeline shares one across the recent-call detail store
    //    and this thread); otherwise generate one here, bounded so a slow/flaky
    //    provider can never dominate the path. Any failure or timeout leaves the
    //    plain-transcript thread untouched.
    let owned_generated = if generated.is_none()
        && matches!(summary_mode, SummaryGenerationMode::GenerateIfMissing)
    {
        super::summary::generate_meeting_summary_bounded(turns, correlation_id.as_deref()).await
    } else {
        None
    };
    let generated = generated.or(owned_generated.as_ref());

    // 3a. Title the thread with the context label (e.g. "Q3 Roadmap") so the
    //     meeting is identifiable in the list (default title is "Chat <date>").
    let context_label = generated.map(|g| g.label.trim()).filter(|l| !l.is_empty());
    if let Some(title) = context_label {
        if let Err(e) = ops::thread_update_title(UpdateConversationThreadTitleRequest {
            thread_id: thread_id.clone(),
            title: title.to_string(),
        })
        .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                "[agent_meetings] failed to set meeting thread title: {e}"
            );
        }
    }

    // 3b. Append the structured summary as a closing message, so the thread ends
    //     with the headline / key points / action items.
    if let Some(g) = generated {
        let summary_body = super::summary::format_summary_markdown(&g.summary, &g.label);
        let summary_msg = ConversationMessageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            content: summary_body,
            message_type: "system".to_string(),
            extra_metadata: serde_json::Value::Null,
            sender: "system".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let summary_req = AppendConversationMessageRequest {
            thread_id: thread_id.clone(),
            message: summary_msg,
        };
        if let Err(e) = ops::message_append(summary_req).await {
            let message = format!("[agent_meetings] failed to append summary message: {e}");
            if matches!(append_mode, ThreadAppendMode::Strict) {
                return Err(message);
            }
            tracing::warn!(
                thread_id = %thread_id,
                "{message}"
            );
        }
    }

    tracing::info!(
        thread_id = %thread_id,
        turn_count = turns.len(),
        summarized = generated.is_some(),
        "[agent_meetings] meeting thread created"
    );
    Ok(thread_id)
}

pub async fn append_summary_prompt_message(
    thread_id: &str,
    meeting_id: &str,
) -> Result<(), String> {
    use crate::openhuman::memory::{AppendConversationMessageRequest, ConversationMessageRecord};
    use crate::openhuman::threads::ops;

    let content = super::summary::format_summary_prompt_markdown(meeting_id);
    let req = AppendConversationMessageRequest {
        thread_id: thread_id.to_string(),
        message: ConversationMessageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            message_type: "system".to_string(),
            extra_metadata: serde_json::json!({
                "kind": "meeting_summary_prompt",
                "meeting_id": meeting_id,
            }),
            sender: "system".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    };
    ops::message_append(req)
        .await
        .map_err(|e| format!("[agent_meetings] append summary prompt failed: {e}"))?;
    tracing::info!(
        thread_id = %thread_id,
        meeting_id = %meeting_id,
        "[agent_meetings] summary prompt appended"
    );
    Ok(())
}

fn detail_transcript_to_turns(
    detail: &crate::openhuman::meet_agent::store::MeetCallDetail,
) -> Vec<BackendMeetTurn> {
    detail
        .transcript
        .iter()
        .filter(|line| !line.content.trim().is_empty())
        .map(|line| BackendMeetTurn {
            role: if line.role.eq_ignore_ascii_case("assistant") {
                "assistant".to_string()
            } else {
                "user".to_string()
            },
            content: line.content.trim().to_string(),
        })
        .collect()
}

async fn recorded_meeting_duration_ms(meeting_id: &str) -> Result<u64, String> {
    let records = crate::openhuman::meet_agent::store::read_recent(
        crate::openhuman::meet_agent::store::MAX_RECENT_CALLS,
    )
    .await?;
    let Some(record) = records
        .into_iter()
        .find(|record| record.request_id == meeting_id)
    else {
        tracing::warn!(
            meeting_id = %meeting_id,
            "[agent_meetings] no recent call row found for manual summary duration"
        );
        return Ok(0);
    };

    let wall_ms = record.ended_at_ms.saturating_sub(record.started_at_ms);
    let audio_ms = seconds_to_millis(record.listened_seconds + record.spoken_seconds);
    Ok(wall_ms.max(audio_ms))
}

fn seconds_to_millis(seconds: f32) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    let millis = (seconds as f64 * 1000.0).round();
    if millis >= u64::MAX as f64 {
        u64::MAX
    } else {
        millis as u64
    }
}

pub async fn handle_generate_summary(params: Map<String, Value>) -> Result<Value, String> {
    let req: GenerateSummaryRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid generate_summary params: {e}"))?;
    let meeting_id = req.meeting_id.trim();
    if meeting_id.is_empty() {
        return Err("[agent_meetings] meeting_id must not be empty".to_string());
    }

    tracing::info!(
        meeting_id = %meeting_id,
        "[agent_meetings] manual summary requested"
    );

    let detail = crate::openhuman::meet_agent::store::read_detail(meeting_id)
        .await?
        .ok_or_else(|| format!("[agent_meetings] no recorded meeting detail for {meeting_id}"))?;
    let turns = detail_transcript_to_turns(&detail);
    if turns.is_empty() {
        return Err(format!(
            "[agent_meetings] meeting {meeting_id} has no transcript lines to summarize"
        ));
    }

    let generated = super::summary::generate_meeting_summary_bounded(&turns, Some(meeting_id))
        .await
        .ok_or_else(|| format!("[agent_meetings] summary generation failed for {meeting_id}"))?;

    let updated = super::recent_calls::build_detail(meeting_id, &turns, Some(&generated));
    crate::openhuman::meet_agent::store::write_detail(&updated).await?;
    let duration_ms = recorded_meeting_duration_ms(meeting_id).await?;

    let thread_id = create_meeting_thread_with_transcript_with_summary_mode_strict(
        &turns,
        duration_ms,
        Some(meeting_id.to_string()),
        Some(&generated),
        SummaryGenerationMode::UseProvidedOnly,
    )
    .await?;

    serde_json::to_value(GenerateSummaryResponse {
        ok: true,
        thread_id,
    })
    .map_err(|e| format!("[agent_meetings] serialize generate_summary response: {e}"))
}

// ---------------------------------------------------------------------------
// Canonical URL / host helpers (single source of truth)
//
// `calendar.rs` and `upcoming.rs` both call into these instead of carrying
// their own near-duplicate copies. Host matching is STRICT — it parses the URL
// and compares the host against `ALLOWED_HOSTS` exactly (or as a subdomain), so
// a spoofed host like `meet.google.com.attacker.com` is rejected (it would have
// passed a loose `contains("meet.google.com")` check).
// ---------------------------------------------------------------------------

/// `true` when `host` is one of the allowed meeting hosts, either exactly or as
/// a subdomain (e.g. `company.zoom.us` matches `zoom.us`).
fn host_is_allowed(host: &str) -> bool {
    ALLOWED_HOSTS
        .iter()
        .any(|(allowed, _)| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

pub(crate) fn validate_meeting_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("invalid meeting URL: {e}"))?;

    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(format!(
            "invalid meeting URL: scheme `{}` not allowed",
            url.scheme()
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| "invalid meeting URL: missing host".to_string())?;

    if !host_is_allowed(host) {
        return Err(format!(
            "invalid meeting URL: host `{host}` not recognized (supported: Google Meet, Zoom, Teams, Webex)"
        ));
    }

    Ok(url)
}

pub(crate) fn infer_platform(url: &url::Url) -> &'static str {
    let host = url.host_str().unwrap_or("");
    for (allowed, platform) in ALLOWED_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            return platform;
        }
    }
    "gmeet"
}

/// Strict check: does `s` parse as an http(s) URL whose host is an allowed
/// meeting host? This is the single canonical `is_meeting_url` used by both the
/// calendar auto-join subscriber and the upcoming-meetings fetcher. Unlike a
/// loose substring match, this rejects `https://meet.google.com.attacker.com/x`.
pub(crate) fn is_meeting_url(s: &str) -> bool {
    match url::Url::parse(s.trim()) {
        Ok(u) => {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().map(host_is_allowed).unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// Infer the platform slug from a URL string using strict host matching.
/// Returns `None` when the host is not a recognized meeting host.
pub(crate) fn infer_platform_from_url(url_str: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(url_str.trim()).ok()?;
    let host = parsed.host_str()?;
    ALLOWED_HOSTS
        .iter()
        .find(|(allowed, _)| host == *allowed || host.ends_with(&format!(".{allowed}")))
        .map(|(_, platform)| *platform)
}

/// Extract the first strictly-validated meeting URL from a free-form text
/// string (e.g. a calendar `location`/`description` like
/// `"Zoom Meeting: https://zoom.us/j/123"`). Scans whitespace-separated tokens,
/// strips surrounding punctuation (including trailing `.`), and returns the
/// first token that parses as an http(s) URL with an allowed meeting host.
pub(crate) fn extract_url_from_text(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|tok| {
            tok.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '"' | '\'' | '.'
                )
            })
        })
        .find_map(|tok| {
            let parsed = url::Url::parse(tok).ok()?;
            (matches!(parsed.scheme(), "http" | "https")
                && parsed.host_str().map(host_is_allowed).unwrap_or(false))
            .then(|| parsed.to_string())
        })
}

/// Extract a stable calendar event id from a calendar event object map, in the
/// canonical priority order shared by every meeting consumer (the UI
/// `meet_list_upcoming` table, the heartbeat planner, and the calendar
/// auto-join subscriber): `id` → `eventId` → `icalUID`. Returns `None` when the
/// object carries none of these.
pub(crate) fn extract_calendar_event_id(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    map.get("id")
        .or_else(|| map.get("eventId"))
        .or_else(|| map.get("icalUID"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Extract the calendar event id from a Composio trigger payload, which may nest
/// the event resource under `data`. Uses the same `id`/`eventId`/`icalUID`
/// priority as [`extract_calendar_event_id`] so the webhook auto-join path keys
/// per-event policy lookups by the SAME id the UI persists them under.
///
/// The `data` sub-object is checked **first** so the actual calendar event id
/// nested under `data` wins over any top-level `id` that might belong to the
/// Composio trigger wrapper rather than to the calendar event itself. Falls back
/// to the top-level object (events.list shape where `id` is directly on the
/// event).
pub(crate) fn extract_calendar_event_id_from_payload(payload: &Value) -> Option<String> {
    // Prefer nested `data` (Composio webhook trigger shape).
    if let Some(data) = payload.get("data") {
        if let Some(obj) = data.as_object() {
            if let Some(id) = extract_calendar_event_id(obj) {
                return Some(id);
            }
        }
    }
    // Fall back to top-level (events.list shape).
    if let Some(obj) = payload.as_object() {
        extract_calendar_event_id(obj)
    } else {
        None
    }
}

/// Build the `bot:join` Socket.IO payload from a validated request.
///
/// Extracted as a pure function so it can be unit-tested independently of the
/// live socket connection.
fn build_join_payload(
    meet_url: &str,
    display_name: &str,
    platform: &str,
    req: &BackendMeetJoinRequest,
) -> Value {
    let mut payload = json!({
        "meetUrl": meet_url,
        "displayName": display_name,
        "platform": platform,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(agent_name) = &req.agent_name {
            map.insert("agentName".to_string(), json!(agent_name));
        }
        if let Some(system_prompt) = &req.system_prompt {
            map.insert("systemPrompt".to_string(), json!(system_prompt));
        }
        if let Some(mascot_id) = &req.mascot_id {
            map.insert("mascotId".to_string(), json!(mascot_id));
        }
        if let Some(rive_colors) = &req.rive_colors {
            map.insert(
                "riveColors".to_string(),
                json!({
                    "primaryColor": rive_colors.primary_color,
                    "secondaryColor": rive_colors.secondary_color,
                }),
            );
        }
        if let Some(respond_to) = &req.respond_to_participant {
            map.insert("respondToParticipant".to_string(), json!(respond_to));
        }
        if let Some(phrase) = &req.wake_phrase {
            map.insert("wakePhrase".to_string(), json!(phrase));
        }
        if let Some(cid) = &req.correlation_id {
            map.insert("correlationId".to_string(), json!(cid));
        }
        if let Some(lo) = req.listen_only {
            map.insert("listenOnly".to_string(), json!(lo));
        }
    }
    payload
}

/// Pure: extract the reply anchor (`respondToParticipant`) carried by a
/// notification action payload. Returns `None` when absent or blank.
fn anchor_from_action_payload(payload: &Value) -> Option<String> {
    payload
        .get("respondToParticipant")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Pure: build the `agent_meetings_join` param map for an AskEachTime
/// notification action.
///
/// Encapsulates the listen-only decision (reply mode without a known anchor is
/// downgraded to listen-only via [`super::calendar::effective_listen_only`]),
/// the wake phrase, and the `respond_to_participant` anchor wiring so they are
/// unit-testable without a live socket/config.
fn build_notification_join_map(
    action_id: &str,
    meet_url: &str,
    correlation_id: &str,
    display_name: Option<&str>,
    respond_to_participant: Option<&str>,
    config_listen_only_default: bool,
) -> Map<String, Value> {
    let requested_listen_only = match action_id {
        "join_listen" => true,
        "join_active" => false,
        _ => config_listen_only_default,
    };
    // Reply mode needs a known anchor. Without one, downgrade to listen-only
    // (still transcribes + summarizes) instead of replying to every speaker.
    let listen_only = super::calendar::effective_listen_only(
        requested_listen_only,
        respond_to_participant.is_some(),
    );
    if listen_only && !requested_listen_only {
        tracing::warn!(
            action_id = %action_id,
            "[agent_meetings] no reply anchor resolved — forcing listen-only join"
        );
    }

    let mut join = Map::new();
    join.insert("meet_url".to_string(), json!(meet_url));
    join.insert("correlation_id".to_string(), json!(correlation_id));
    join.insert("listen_only".to_string(), json!(listen_only));
    if let Some(name) = display_name {
        join.insert("display_name".to_string(), json!(name));
    }
    if !listen_only {
        // Reply mode: the participant addresses the bot as "Hey Tiny"; the
        // wake phrase is always required (no implicit address).
        join.insert("wake_phrase".to_string(), json!("Hey Tiny"));
        // Anchor replies to the meeting owner so the bot knows who it is
        // answering (empty/absent = respond to everyone).
        if let Some(owner) = respond_to_participant {
            join.insert("respond_to_participant".to_string(), json!(owner));
        }
    }
    join
}

/// Handle `openhuman.agent_meetings_join`.
pub async fn handle_join(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetJoinRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid join params: {e}"))?;

    let normalized_url =
        validate_meeting_url(&req.meet_url).map_err(|e| format!("[agent_meetings] {e}"))?;

    let display_name = match &req.display_name {
        Some(name) => validate_display_name(name).map_err(|e| format!("[agent_meetings] {e}"))?,
        None => "Tiny".to_string(),
    };

    let inferred = infer_platform(&normalized_url);
    let platform = match req.platform.as_deref() {
        Some(p) if p != inferred => {
            return Err(format!(
                "[agent_meetings] platform mismatch: URL implies `{inferred}` but `{p}` was supplied"
            ));
        }
        Some(p) => p,
        None => inferred,
    };

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        meet_url_host = %normalized_url.host_str().unwrap_or(""),
        platform = %platform,
        display_name_len = display_name.len(),
        "[agent_meetings] emitting bot:join"
    );

    let join_payload = build_join_payload(normalized_url.as_str(), &display_name, platform, &req);

    mgr.emit("bot:join", join_payload)
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    // Snapshot join context so the post-call recent-calls record can show who
    // launched the bot, into which meeting. Keyed by correlation_id; consumed
    // when the `BackendMeetTranscript` event arrives at call-end. No-op when
    // the caller didn't supply a correlation_id.
    super::recent_calls::remember_join(
        req.correlation_id.as_deref(),
        super::recent_calls::JoinMeta {
            meet_url: normalized_url.to_string(),
            // "Your Name in This Meeting" — the human who launched the bot and
            // whom it answers to. This is the owner shown in the recent-calls list.
            owner_display_name: req.respond_to_participant.clone().unwrap_or_default(),
            // The bot's tile name in the meeting (persona display name).
            bot_display_name: display_name.clone(),
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
    );

    // Active mode (listen_only = false, the modal's "respond when addressed"
    // toggle) enables in-call agency for just this meeting, so the toggle
    // "just works" without flipping the global config. Passive joins leave
    // the meeting unmarked (default: listen-only / transcribe-only).
    if req.listen_only == Some(false) {
        super::in_call::mark_meeting_active(req.correlation_id.as_deref()).await;
    }

    let response = BackendMeetJoinResponse {
        ok: true,
        meet_url: normalized_url.to_string(),
        platform: platform.to_string(),
    };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response).map_err(|e| format!("[agent_meetings] serialize: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_leave`.
pub async fn handle_leave(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetLeaveRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid leave params: {e}"))?;

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    let reason = req.reason.unwrap_or_else(|| "requested".to_string());

    tracing::info!(reason = %reason, "[agent_meetings] emitting bot:leave");

    mgr.emit("bot:leave", json!({ "reason": reason }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_harness_response`.
pub async fn handle_harness_response(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetHarnessResponseRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid harness_response params: {e}"))?;

    if req.result.trim().is_empty() {
        return Err("[agent_meetings] result must not be empty".to_string());
    }

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        result_len = req.result.len(),
        "[agent_meetings] emitting bot:harness:response"
    );

    mgr.emit("bot:harness:response", json!({ "result": req.result }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_speak`.
pub async fn handle_speak(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetSpeakRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid speak request: {e}"))?;

    if req.text.trim().is_empty() {
        return Err("[agent_meetings] text must not be empty".to_string());
    }

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        text_len = req.text.len(),
        correlation_id = ?req.correlation_id,
        "[agent_meetings] emitting bot:speak"
    );

    let mut speak_payload = json!({ "text": req.text });
    if let Some(map) = speak_payload.as_object_mut() {
        if let Some(cid) = &req.correlation_id {
            map.insert("correlationId".to_string(), json!(cid));
        }

        // The RPC-driven `agent_meetings_speak` delivers the agent's spoken
        // reply, so tag it terminal (`kind="reply"`) to match the in-call reply
        // path (`in_call::emit_bot_speak`) — both `bot:speak` emitters now carry
        // the same `kind` field so the backend mascot can settle to idle after a
        // real reply. Additive; older backends ignore it.
        map.insert("kind".to_string(), json!("reply"));
    }

    mgr.emit("bot:speak", speak_payload)
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_notification_action` — a click on one
/// of the calendar auto-join notification buttons (issue #3507).
///
/// Actions:
/// - `join_listen`  → join muted (transcript-only).
/// - `join_active`  → join in reply mode with the "Hey Tiny" wake phrase.
/// - `skip`         → mark the meeting session Ended; no join.
/// - `always_join`  → persist `auto_join_policy = Always`, then join with
///   the configured `listen_only_default`.
///
/// `payload` carries `{ meetingId, meetUrl, title }` from the notification
/// plus an optional user-edited `displayName`.
pub async fn handle_notification_action(params: Map<String, Value>) -> Result<Value, String> {
    let action_id = params
        .get("action_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if action_id.is_empty() {
        return Err("[agent_meetings] action_id is required".to_string());
    }
    let payload = params.get("payload").cloned().unwrap_or(Value::Null);
    let meeting_id = payload
        .get("meetingId")
        .and_then(|v| v.as_str())
        .map(String::from);
    let meet_url = payload
        .get("meetUrl")
        .and_then(|v| v.as_str())
        .map(String::from);
    let display_name = payload
        .get("displayName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // Reply anchor carried from the calendar notification (issue: gmeet
    // auto-join anchor). Falls back to the signed-in account identity so a
    // notification raised before the anchor wiring still knows who to reply to.
    let respond_to_participant = anchor_from_action_payload(&payload).or_else(|| {
        crate::openhuman::app_state::peek_cached_current_user_identity()
            .and_then(|i| i.name)
            .map(|n| n.trim().to_string())
            .filter(|s| !s.is_empty())
    });

    tracing::info!(
        action_id = %action_id,
        meeting_id = ?meeting_id,
        has_meet_url = meet_url.is_some(),
        "[agent_meetings] notification action received"
    );

    match action_id.as_str() {
        "skip" => {
            if let Some(id) = &meeting_id {
                match crate::openhuman::config::ops::load_config_with_timeout().await {
                    Ok(config) => {
                        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                        if let Err(e) = super::store::update_session_status(
                            &config,
                            id,
                            MeetingSessionStatus::Ended,
                            now_ms,
                        ) {
                            tracing::debug!("[agent_meetings] skip: session update failed: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::debug!("[agent_meetings] skip: config load failed: {e}");
                    }
                }
            }
            let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
            outcome.into_cli_compatible_json()
        }
        "join_listen" | "join_active" | "always_join" => {
            let meet_url = meet_url
                .ok_or_else(|| "[agent_meetings] payload.meetUrl is required".to_string())?;
            let config = crate::openhuman::config::ops::load_config_with_timeout().await?;

            if action_id == "always_join" {
                let mut cfg = config.clone();
                cfg.meet.auto_join_policy =
                    crate::openhuman::config::schema::AutoJoinPolicy::Always;
                if let Err(e) = cfg.save().await {
                    // Join anyway — the policy flip failing must not block
                    // the join the user just asked for.
                    tracing::warn!("[agent_meetings] persisting always-join policy failed: {e}");
                }
            }

            let correlation_id = meeting_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let join = build_notification_join_map(
                &action_id,
                &meet_url,
                &correlation_id,
                display_name.as_deref(),
                respond_to_participant.as_deref(),
                config.meet.listen_only_default,
            );

            handle_join(join).await
        }
        other => Err(format!("[agent_meetings] unknown action_id: {other}")),
    }
}

/// Handle `openhuman.meet_list_upcoming` — list upcoming calendar meetings that
/// have a conferencing link, fetching from Composio's Google Calendar integration.
///
/// Returns an empty meetings list (ok=true) when:
/// - No calendar is connected.
/// - The user is not signed in to the backend.
/// - All connections are inactive.
///
/// Returns an error string only for hard failures (bad params, config load).
pub async fn handle_list_upcoming(params: Map<String, Value>) -> Result<Value, String> {
    use super::types::{ListUpcomingRequest, ListUpcomingResponse};
    use super::upcoming::{fetch_upcoming_meetings, DEFAULT_LIMIT, DEFAULT_LOOKAHEAD_MINUTES};

    let req: ListUpcomingRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[meet:upcoming] invalid params: {e}"))?;

    let lookahead_minutes = req.lookahead_minutes.unwrap_or(DEFAULT_LOOKAHEAD_MINUTES);
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);

    tracing::debug!(
        lookahead_minutes,
        limit,
        "[meet:upcoming] handle_list_upcoming called"
    );

    // Load config to get the auto_join_policy and composio settings.
    let config = crate::openhuman::config::ops::load_config_with_timeout()
        .await
        .map_err(|e| format!("[meet:upcoming] config load failed: {e}"))?;

    // The global fallback policy string (used when no per-event or per-platform override exists).
    let global_policy = auto_join_policy_to_str(&config.meet.auto_join_policy);

    let mut meetings = fetch_upcoming_meetings(&config, lookahead_minutes, limit, global_policy)
        .await
        .map_err(|e| format!("[meet:upcoming] fetch failed: {e}"))?;

    // Phase 3: resolve effective per-event policy overrides.
    //
    // Batch-load every meeting's per-event override in ONE SQLite connection
    // (single schema migration) up front, then resolve each meeting's effective
    // policy fully in-memory. The previous per-meeting `get_event_policy` call
    // opened a fresh connection AND re-ran the full schema migration once per
    // meeting — up to ~100× per RPC.
    let event_overrides = {
        let event_ids: Vec<&str> = meetings
            .iter()
            .map(|m| m.calendar_event_id.as_str())
            .collect();
        super::store::get_event_policies_batch(&config, &event_ids).unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                "[meet:upcoming] batch policy fetch failed — falling back to global/per-platform only"
            );
            HashMap::new()
        })
    };
    for meeting in &mut meetings {
        let platform = meeting.platform.as_deref();
        let event_id = Some(meeting.calendar_event_id.as_str());
        let effective = resolve_effective_join_policy_with_overrides(
            event_id,
            platform,
            &config,
            &event_overrides,
        );
        if effective != meeting.join_policy {
            tracing::debug!(
                calendar_event_id = %meeting.calendar_event_id,
                global = %meeting.join_policy,
                effective = %effective,
                "[meet:upcoming] per-event/platform policy override applied"
            );
            meeting.join_policy = effective;
        }
    }

    tracing::info!(
        count = meetings.len(),
        global_policy,
        "[meet:upcoming] returning meetings"
    );

    let response = ListUpcomingResponse { ok: true, meetings };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response)
            .map_err(|e| format!("[meet:upcoming] serialize failed: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.meet_set_event_policy` — persist a per-event join-policy
/// override for a specific calendar event.
pub async fn handle_set_event_policy(params: Map<String, Value>) -> Result<Value, String> {
    use super::types::{SetEventPolicyRequest, SetEventPolicyResponse};

    let req: SetEventPolicyRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[meet:set_event_policy] invalid params: {e}"))?;

    let policy = req.policy.trim();
    if !matches!(policy, "auto" | "ask" | "skip") {
        return Err(format!(
            "[meet:set_event_policy] invalid policy: {policy} (valid: auto, ask, skip)"
        ));
    }

    let calendar_event_id = req.calendar_event_id.trim().to_string();
    if calendar_event_id.is_empty() {
        return Err("[meet:set_event_policy] calendar_event_id must not be empty".to_string());
    }

    tracing::debug!(
        calendar_event_id = %calendar_event_id,
        policy = %policy,
        "[meet:set_event_policy] persisting policy override"
    );

    let config = crate::openhuman::config::ops::load_config_with_timeout()
        .await
        .map_err(|e| format!("[meet:set_event_policy] config load failed: {e}"))?;

    super::store::set_event_policy(&config, &calendar_event_id, policy)
        .map_err(|e| format!("[meet:set_event_policy] store failed: {e}"))?;

    tracing::info!(
        calendar_event_id = %calendar_event_id,
        policy = %policy,
        "[meet:set_event_policy] policy stored"
    );

    let response = SetEventPolicyResponse { ok: true };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response)
            .map_err(|e| format!("[meet:set_event_policy] serialize failed: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.meet_get_event_policies` — retrieve stored per-event
/// join-policy overrides for a batch of calendar event IDs.
pub async fn handle_get_event_policies(params: Map<String, Value>) -> Result<Value, String> {
    use super::types::{GetEventPoliciesRequest, GetEventPoliciesResponse};

    let req: GetEventPoliciesRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[meet:get_event_policies] invalid params: {e}"))?;

    tracing::debug!(
        count = req.calendar_event_ids.len(),
        "[meet:get_event_policies] fetching policies"
    );

    let config = crate::openhuman::config::ops::load_config_with_timeout()
        .await
        .map_err(|e| format!("[meet:get_event_policies] config load failed: {e}"))?;

    let id_refs: Vec<&str> = req.calendar_event_ids.iter().map(String::as_str).collect();
    let policies = super::store::get_event_policies_batch(&config, &id_refs)
        .map_err(|e| format!("[meet:get_event_policies] store failed: {e}"))?;

    tracing::debug!(
        found = policies.len(),
        "[meet:get_event_policies] returning policies"
    );

    let response = GetEventPoliciesResponse { ok: true, policies };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response)
            .map_err(|e| format!("[meet:get_event_policies] serialize failed: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_google_meet_url() {
        validate_meeting_url("https://meet.google.com/abc-defg-hij").unwrap();
    }

    #[test]
    fn accepts_zoom_url() {
        validate_meeting_url("https://zoom.us/j/123456789").unwrap();
        validate_meeting_url("https://company.zoom.us/j/123456789").unwrap();
    }

    #[test]
    fn accepts_teams_url() {
        validate_meeting_url("https://teams.microsoft.com/l/meetup-join/abc").unwrap();
    }

    #[test]
    fn accepts_webex_url() {
        validate_meeting_url("https://meet.webex.com/meet/abc").unwrap();
        validate_meeting_url("https://company.webex.com/meet/abc").unwrap();
    }

    #[test]
    fn rejects_unknown_host() {
        assert!(validate_meeting_url("https://example.com/meeting").is_err());
    }

    // ── strict host matching (anti-spoof) ───────────────────────

    #[test]
    fn validate_rejects_spoofed_suffix_host() {
        // A loose `contains("meet.google.com")` check would let these through.
        assert!(validate_meeting_url("https://meet.google.com.attacker.com/x").is_err());
        assert!(validate_meeting_url("https://zoom.us.evil.example/x").is_err());
        assert!(validate_meeting_url("https://notzoom.us/x").is_err());
    }

    #[test]
    fn is_meeting_url_strict_rejects_spoofed_host() {
        assert!(!is_meeting_url("https://meet.google.com.attacker.com/x"));
        assert!(!is_meeting_url("https://evilzoom.us.attacker.com/j/1"));
        // Bare host with no scheme is not a usable meeting URL.
        assert!(!is_meeting_url("meet.google.com/abc"));
    }

    #[test]
    fn is_meeting_url_accepts_allowed_hosts_and_subdomains() {
        assert!(is_meeting_url("https://meet.google.com/abc-defg-hij"));
        assert!(is_meeting_url("https://zoom.us/j/123"));
        assert!(is_meeting_url("https://company.zoom.us/j/123"));
        assert!(is_meeting_url(
            "https://teams.microsoft.com/l/meetup-join/abc"
        ));
        assert!(is_meeting_url("https://meet.webex.com/meet/abc"));
    }

    #[test]
    fn infer_platform_from_url_strict() {
        assert_eq!(
            infer_platform_from_url("https://company.zoom.us/j/1"),
            Some("zoom")
        );
        assert_eq!(
            infer_platform_from_url("https://meet.google.com/abc"),
            Some("gmeet")
        );
        // Spoofed host must not infer a platform.
        assert!(infer_platform_from_url("https://meet.google.com.attacker.com/x").is_none());
        assert!(infer_platform_from_url("https://example.com/x").is_none());
    }

    #[test]
    fn extract_url_from_text_strict() {
        assert_eq!(
            extract_url_from_text("Zoom Meeting: https://zoom.us/j/123"),
            Some("https://zoom.us/j/123".to_string())
        );
        // Spoofed host embedded in free-form text is rejected.
        assert!(extract_url_from_text("Join https://meet.google.com.attacker.com/x now").is_none());
    }

    #[test]
    fn extract_calendar_event_id_priority() {
        let map = json!({ "id": "real-id", "eventId": "ev", "icalUID": "ical" });
        assert_eq!(
            extract_calendar_event_id(map.as_object().unwrap()).as_deref(),
            Some("real-id")
        );
        let map = json!({ "eventId": "ev", "icalUID": "ical" });
        assert_eq!(
            extract_calendar_event_id(map.as_object().unwrap()).as_deref(),
            Some("ev")
        );
        let map = json!({ "summary": "no id" });
        assert!(extract_calendar_event_id(map.as_object().unwrap()).is_none());
    }

    #[test]
    fn extract_calendar_event_id_from_payload_handles_nested_data() {
        let payload = json!({ "data": { "id": "nested-id" } });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("nested-id")
        );
        let payload = json!({ "id": "top-id" });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("top-id")
        );
    }

    // ── nested data preferred over top-level (finding #4) ──────

    #[test]
    fn extract_calendar_event_id_from_payload_prefers_nested_data_when_both_present() {
        // A Composio trigger wrapper may carry its own top-level `id` (trigger
        // metadata) while the actual calendar event id is under `data.id`.
        // The nested value must always win.
        let payload = json!({
            "id": "trigger-wrapper-id",
            "data": { "id": "calendar-event-id-real" }
        });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("calendar-event-id-real"),
            "nested data.id must win over top-level trigger wrapper id"
        );
    }

    #[test]
    fn extract_calendar_event_id_from_payload_uses_nested_event_id_fallback() {
        // data.eventId when data.id is absent.
        let payload = json!({ "id": "outer-id", "data": { "eventId": "ev-nested-456" } });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("ev-nested-456"),
            "nested data.eventId must win over top-level id"
        );
    }

    #[test]
    fn extract_calendar_event_id_from_payload_uses_nested_ical_uid() {
        // data.icalUID as last resort in nested data.
        let payload = json!({
            "id": "outer-id",
            "data": { "icalUID": "ical-uid-nested@calendar.google.com" }
        });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("ical-uid-nested@calendar.google.com"),
            "nested data.icalUID must win over top-level id"
        );
    }

    #[test]
    fn extract_calendar_event_id_from_payload_falls_back_to_top_level_when_data_has_no_id() {
        // data is present but has no id fields → fall back to top-level.
        let payload = json!({ "id": "top-id", "data": { "summary": "No id here" } });
        assert_eq!(
            extract_calendar_event_id_from_payload(&payload).as_deref(),
            Some("top-id"),
            "top-level id used when data has no id fields"
        );
    }

    // ── batch policy resolution ─────────────────────────────────

    #[test]
    fn resolve_with_overrides_tiers() {
        use crate::openhuman::config::schema::AutoJoinPolicy;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut config = crate::openhuman::config::Config::default();
        config.workspace_dir = dir.path().to_path_buf();
        config
            .meet
            .platform_auto_join_policies
            .insert("zoom".to_string(), AutoJoinPolicy::Always);

        let mut overrides = HashMap::new();
        overrides.insert("evt-1".to_string(), "skip".to_string());

        // Tier 1: per-event override wins.
        assert_eq!(
            resolve_effective_join_policy_with_overrides(
                Some("evt-1"),
                Some("zoom"),
                &config,
                &overrides
            ),
            "skip"
        );
        // Tier 2: no override for this event → per-platform.
        assert_eq!(
            resolve_effective_join_policy_with_overrides(
                Some("evt-2"),
                Some("zoom"),
                &config,
                &overrides
            ),
            "auto"
        );
        // Tier 3: no override, unknown platform → global default ("ask").
        assert_eq!(
            resolve_effective_join_policy_with_overrides(
                Some("evt-2"),
                Some("gmeet"),
                &config,
                &overrides
            ),
            "ask"
        );
    }

    #[tokio::test]
    async fn notification_action_requires_action_id() {
        let err = handle_notification_action(Map::new()).await.unwrap_err();
        assert!(err.contains("action_id"));
    }

    #[tokio::test]
    async fn notification_action_rejects_unknown_action() {
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("explode"));
        let err = handle_notification_action(params).await.unwrap_err();
        assert!(err.contains("unknown action_id"));
    }

    // ── anchor_from_action_payload ──────────────────────────────

    #[test]
    fn anchor_extracted_from_payload() {
        let payload = json!({ "respondToParticipant": "Shanu Goyanka" });
        assert_eq!(
            anchor_from_action_payload(&payload).as_deref(),
            Some("Shanu Goyanka")
        );
    }

    #[test]
    fn anchor_none_when_absent_or_blank() {
        assert!(anchor_from_action_payload(&json!({})).is_none());
        assert!(anchor_from_action_payload(&json!({ "respondToParticipant": "  " })).is_none());
    }

    // ── build_notification_join_map ─────────────────────────────

    #[test]
    fn join_map_listen_only_action_has_no_anchor_or_wake() {
        let join = build_notification_join_map(
            "join_listen",
            "https://meet.google.com/abc",
            "corr-1",
            Some("Tiny"),
            Some("Shanu"),
            false,
        );
        assert_eq!(join["listen_only"], json!(true));
        assert_eq!(join["display_name"], json!("Tiny"));
        // listen-only never carries wake/anchor
        assert!(!join.contains_key("wake_phrase"));
        assert!(!join.contains_key("respond_to_participant"));
    }

    #[test]
    fn join_map_active_with_anchor_carries_wake_and_anchor() {
        let join = build_notification_join_map(
            "join_active",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            false,
        );
        assert_eq!(join["listen_only"], json!(false));
        assert_eq!(join["wake_phrase"], json!("Hey Tiny"));
        assert_eq!(join["respond_to_participant"], json!("Shanu"));
        assert!(!join.contains_key("display_name"));
    }

    #[test]
    fn join_map_active_without_anchor_downgrades_to_listen_only() {
        let join = build_notification_join_map(
            "join_active",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            None,
            false,
        );
        // No anchor → forced listen-only, no wake/anchor emitted.
        assert_eq!(join["listen_only"], json!(true));
        assert!(!join.contains_key("wake_phrase"));
        assert!(!join.contains_key("respond_to_participant"));
    }

    #[test]
    fn join_map_always_join_uses_config_default() {
        // always_join + config default reply (false) + anchor → reply mode.
        let reply = build_notification_join_map(
            "always_join",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            false,
        );
        assert_eq!(reply["listen_only"], json!(false));
        assert_eq!(reply["respond_to_participant"], json!("Shanu"));

        // always_join + config default listen-only (true) → listen-only.
        let passive = build_notification_join_map(
            "always_join",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            true,
        );
        assert_eq!(passive["listen_only"], json!(true));
        assert!(!passive.contains_key("respond_to_participant"));
    }

    #[tokio::test]
    async fn notification_action_join_requires_meet_url() {
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("join_listen"));
        params.insert("payload".to_string(), json!({ "meetingId": "m-1" }));
        let err = handle_notification_action(params).await.unwrap_err();
        assert!(err.contains("meetUrl"));
    }

    #[tokio::test]
    async fn notification_action_skip_without_meeting_id_is_ok() {
        // No meetingId → nothing to update; must succeed without touching
        // config or the session store.
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("skip"));
        let value = handle_notification_action(params).await.unwrap();
        assert_eq!(value.get("ok"), Some(&json!(true)));
    }

    #[test]
    fn infers_platform_from_host() {
        let url = url::Url::parse("https://meet.google.com/abc-defg-hij").unwrap();
        assert_eq!(infer_platform(&url), "gmeet");

        let url = url::Url::parse("https://zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");

        let url = url::Url::parse("https://teams.microsoft.com/l/meetup").unwrap();
        assert_eq!(infer_platform(&url), "teams");

        let url = url::Url::parse("https://meet.webex.com/meet/abc").unwrap();
        assert_eq!(infer_platform(&url), "webex");

        let url = url::Url::parse("https://company.zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");
    }

    #[test]
    fn transcript_turns_convert_to_chat_batch() {
        let batch = transcript_turns_to_chat_batch(
            &[
                BackendMeetTurn {
                    role: "user".to_string(),
                    content: "[Alice] OpenHuman, summarize this.".to_string(),
                },
                BackendMeetTurn {
                    role: "assistant".to_string(),
                    content: "Sure, here is the summary.".to_string(),
                },
            ],
            1_000,
        )
        .expect("batch");

        assert_eq!(batch.platform, "backend_meet");
        assert_eq!(batch.messages.len(), 2);
        assert_eq!(batch.messages[0].author, "Meeting participant");
        assert_eq!(batch.messages[1].author, "Tiny");
        assert!(batch.messages[0].text.contains("summarize"));
    }

    #[tokio::test]
    async fn join_fails_when_socket_not_connected() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"meet_url": "https://meet.google.com/abc-defg-hij"}))
                .unwrap();
        let result = handle_join(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("socket not connected"));
    }

    #[tokio::test]
    async fn harness_response_rejects_empty_result() {
        let params: Map<String, Value> = serde_json::from_value(json!({"result": "   "})).unwrap();
        let result = handle_harness_response(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    // --- build_join_payload ---

    fn minimal_req(meet_url: &str) -> BackendMeetJoinRequest {
        serde_json::from_value(json!({ "meet_url": meet_url })).unwrap()
    }

    #[test]
    fn build_join_payload_minimal() {
        let req = minimal_req("https://meet.google.com/abc-defg-hij");
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["meetUrl"], "https://meet.google.com/abc-defg-hij");
        assert_eq!(payload["displayName"], "OpenHuman");
        assert_eq!(payload["platform"], "gmeet");
        assert!(payload.get("agentName").is_none());
        assert!(payload.get("systemPrompt").is_none());
        assert!(payload.get("mascotId").is_none());
        assert!(payload.get("riveColors").is_none());
        assert!(payload.get("respondToParticipant").is_none());
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_respond_to_participant() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "respond_to_participant": "Alice"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["respondToParticipant"], "Alice");
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_wake_phrase() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["wakePhrase"], "Hey bot");
        assert!(payload.get("respondToParticipant").is_none());
    }

    #[test]
    fn build_join_payload_with_all_optional_fields() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://teams.microsoft.com/l/meet/abc",
            "agent_name": "MyBot",
            "system_prompt": "You are a helpful assistant.",
            "mascot_id": "yellow",
            "rive_colors": {
                "primary_color": "#ff0000",
                "secondary_color": "#00ff00"
            },
            "respond_to_participant": "Bob",
            "wake_phrase": "Hello bot"
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://teams.microsoft.com/l/meet/abc",
            "MyBot",
            "teams",
            &req,
        );
        assert_eq!(payload["agentName"], "MyBot");
        assert_eq!(payload["systemPrompt"], "You are a helpful assistant.");
        assert_eq!(payload["mascotId"], "yellow");
        assert_eq!(payload["riveColors"]["primaryColor"], "#ff0000");
        assert_eq!(payload["riveColors"]["secondaryColor"], "#00ff00");
        assert_eq!(payload["respondToParticipant"], "Bob");
        assert_eq!(payload["wakePhrase"], "Hello bot");
    }

    #[test]
    fn join_request_fields_deserialize_correctly() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "respond_to_participant": "Alice",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        assert_eq!(req.respond_to_participant.as_deref(), Some("Alice"));
        assert_eq!(req.wake_phrase.as_deref(), Some("Hey bot"));
    }

    #[test]
    fn join_request_optional_fields_absent_by_default() {
        let req: BackendMeetJoinRequest =
            serde_json::from_value(json!({ "meet_url": "https://meet.google.com/abc-defg-hij" }))
                .unwrap();
        assert!(req.respond_to_participant.is_none());
        assert!(req.wake_phrase.is_none());
        assert!(req.agent_name.is_none());
        assert!(req.system_prompt.is_none());
        assert!(req.mascot_id.is_none());
        assert!(req.rive_colors.is_none());
    }

    #[test]
    fn build_join_payload_with_correlation_id() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "correlation_id": "meeting-123"
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["correlationId"], "meeting-123");
    }

    #[test]
    fn build_join_payload_with_listen_only() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "listen_only": true
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["listenOnly"], true);
    }

    #[test]
    fn build_join_payload_correlation_and_listen_only_absent_by_default() {
        let req = minimal_req("https://meet.google.com/abc-defg-hij");
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert!(payload.get("correlationId").is_none());
        assert!(payload.get("listenOnly").is_none());
    }

    #[test]
    fn join_request_correlation_and_listen_only_deserialize() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "correlation_id": "sess-456",
            "listen_only": true
        }))
        .unwrap();
        assert_eq!(req.correlation_id.as_deref(), Some("sess-456"));
        assert_eq!(req.listen_only, Some(true));
    }

    #[test]
    fn transcript_turns_empty_returns_none() {
        let result = transcript_turns_to_chat_batch(&[], 1_000);
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_all_blank_content_returns_none() {
        let result = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "   ".to_string(),
            }],
            1_000,
        );
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_zero_duration_no_panic() {
        let batch = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            0,
        )
        .expect("batch");
        assert_eq!(batch.messages.len(), 1);
    }

    #[test]
    fn rive_colors_deserialize() {
        use crate::openhuman::agent_meetings::types::RiveColors;
        let rc: RiveColors =
            serde_json::from_value(json!({"primary_color": "#abc", "secondary_color": "#def"}))
                .unwrap();
        assert_eq!(rc.primary_color.as_deref(), Some("#abc"));
        assert_eq!(rc.secondary_color.as_deref(), Some("#def"));
    }

    // ── policy resolution tiers ─────────────────────────────────

    #[test]
    fn policy_resolution_tiers() {
        use crate::openhuman::config::schema::AutoJoinPolicy;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut config = crate::openhuman::config::Config::default();
        config.workspace_dir = dir.path().to_path_buf();

        // Global default is AskEachTime → "ask".
        assert_eq!(resolve_effective_join_policy(None, None, &config), "ask");

        // Per-platform override for "zoom" → "auto".
        config
            .meet
            .platform_auto_join_policies
            .insert("zoom".to_string(), AutoJoinPolicy::Always);
        assert_eq!(
            resolve_effective_join_policy(None, Some("zoom"), &config),
            "auto"
        );
        // Other platforms still fall through to global.
        assert_eq!(
            resolve_effective_join_policy(None, Some("gmeet"), &config),
            "ask"
        );

        // Per-event override wins over per-platform.
        crate::openhuman::agent_meetings::store::set_event_policy(&config, "evt-zoom-1", "skip")
            .unwrap();
        assert_eq!(
            resolve_effective_join_policy(Some("evt-zoom-1"), Some("zoom"), &config),
            "skip"
        );

        // Different event still gets per-platform.
        assert_eq!(
            resolve_effective_join_policy(Some("evt-zoom-2"), Some("zoom"), &config),
            "auto"
        );
    }

    struct CountingProvider {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::openhuman::inference::provider::Provider for CountingProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("{\"label\":\"Policy Test\",\"headline\":\"Done\",\"key_points\":[],\"action_items\":[]}"
                .to_string())
        }
    }

    struct EnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set_workspace(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
                None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
            }
        }
    }

    #[tokio::test]
    async fn recorded_meeting_duration_uses_recent_call_row() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        let _env = EnvGuard::set_workspace(tmp.path());

        crate::openhuman::meet_agent::store::append_record(
            &crate::openhuman::meet_agent::store::MeetCallRecord {
                request_id: "duration-call".to_string(),
                meet_url: "https://meet.google.com/abc-defg-hij".to_string(),
                bot_display_name: "OpenHuman".to_string(),
                owner_display_name: "Alice".to_string(),
                started_at_ms: 10_000,
                ended_at_ms: 130_000,
                listened_seconds: 30.0,
                spoken_seconds: 2.0,
                turn_count: 1,
                participants: vec!["Alice".to_string()],
            },
        )
        .await
        .expect("record call row");

        let duration_ms = recorded_meeting_duration_ms("duration-call")
            .await
            .expect("duration reads from recent calls");

        assert_eq!(duration_ms, 120_000);
    }

    #[tokio::test]
    async fn thread_creation_rejects_empty_transcript_turns() {
        let err = create_meeting_thread_with_transcript_with_summary_mode(
            &[],
            60_000,
            Some("empty-transcript".to_string()),
            None,
            SummaryGenerationMode::UseProvidedOnly,
        )
        .await
        .expect_err("empty transcript should not return a successful empty thread ID");

        assert!(
            err.contains("without transcript turns"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn thread_creation_use_provided_only_does_not_generate_missing_summary() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        let _env = EnvGuard::set_workspace(tmp.path());

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let _provider =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                std::sync::Arc::new(CountingProvider {
                    calls: calls.clone(),
                }),
            );

        let turns = vec![BackendMeetTurn {
            role: "user".to_string(),
            content: "[00:01] [Alice] ship it".to_string(),
        }];

        let thread_id = create_meeting_thread_with_transcript_with_summary_mode(
            &turns,
            60_000,
            Some("policy-never".to_string()),
            None,
            SummaryGenerationMode::UseProvidedOnly,
        )
        .await
        .expect("thread created without generated summary");

        assert!(!thread_id.is_empty());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "UseProvidedOnly must not call the summarization provider"
        );
    }
}
